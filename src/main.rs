use std::io::Write;
use std::num::NonZeroUsize;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode};
use log::{debug, error, info, log_enabled, Level};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::time::sleep;

use icy_metadata::{IcyHeaders, IcyMetadataReader, RequestIcyMetadata};
use rodio;
use stream_download::http::reqwest::Client;
use stream_download::http::HttpStream;
use stream_download::storage::bounded::BoundedStorageProvider;
use stream_download::storage::memory::MemoryStorageProvider;
use stream_download::{Settings, StreamDownload};

// Data structures for API response
#[derive(Debug, Deserialize)]
struct SongInfo {
    TITLE: String,
    ARTIST: String,
    ALBUM: String,
    YEAR: String,
    CIRCLE: String,
}

#[derive(Debug, Deserialize)]
struct SongTimes {
    DURATION: u32,
    PLAYED: u32,
}

#[derive(Debug, Deserialize)]
struct SongData {
    RATING: String,
}

#[derive(Debug, Deserialize)]
struct Misc {
    ALBUMART: Option<String>, // ALBUMART may be empty, so use Option
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    SONGINFO: SongInfo,
    SONGTIMES: SongTimes,
    SONGDATA: SongData,
    MISC: Misc,
}

// Displays a loading animation for a given duration
async fn loading_animation(duration: Duration) {
    let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let mut i = 0;

    let start = Instant::now();
    while start.elapsed() < duration {
        print!("\rLoading {}", spinner[i % spinner.len()]);
        std::io::stdout().flush().unwrap();
        i += 1;
        sleep(Duration::from_millis(100)).await;
    }
    print!("\rLoading Done!          \n");
}

// Fetches metadata from the API and displays it
async fn fetch_metadata() -> Result<(), Box<dyn std::error::Error>> {
    // Clear the terminal screen
    print!("\x1B[2J\x1B[H"); // ANSI escape sequence: clear screen and move cursor to top-left
    std::io::stdout().flush().unwrap();

    let response = reqwest::get("https://gensokyoradio.net/api/station/playing/")
        .await?
        .json::<ApiResponse>()
        .await?;

    // Check if ALBUMART exists
    if let Some(album_art) = &response.MISC.ALBUMART {
        if !album_art.is_empty() {
            let album_art_url = format!("https://gensokyoradio.net/images/albums/500/{}", album_art);

            // Download album art image
            let image_data = reqwest::get(&album_art_url).await?.bytes().await?;

            // Use wezterm imgcat to display the image
            let mut wezterm = Command::new("wezterm")
                .arg("imgcat")
                .stdin(Stdio::piped())
                .spawn()
                .expect("Failed to spawn wezterm imgcat");

            if let Some(mut stdin) = wezterm.stdin.take() {
                stdin.write_all(&image_data)?;
            }

            wezterm.wait()?;
        }
    }

    // Print song information
    println!("Title: {}", response.SONGINFO.TITLE);
    println!("Artist: {}", response.SONGINFO.ARTIST);
    println!("Album: {}", response.SONGINFO.ALBUM);
    println!("Year: {}", response.SONGINFO.YEAR);
    println!("Circle: {}", response.SONGINFO.CIRCLE);
    println!("Rating: {}", response.SONGDATA.RATING);

    // Get song duration and played time
    let duration = response.SONGTIMES.DURATION;
    let mut played = response.SONGTIMES.PLAYED;

    // Start time tracking
    let start_time = Instant::now();

    // Dynamically update progress bar until metadata is updated
    while played < duration {
        let bar_length = 50; // Length of the progress bar
        let filled_length = (played as f32 / duration as f32 * bar_length as f32).round() as usize;

        // Build the progress bar
        let bar = format!(
            "[{}{}]",
            "█".repeat(filled_length),
            "░".repeat(bar_length - filled_length)
        );

        // Format time
        let elapsed_time = played;
        let total_time = duration;
        let elapsed_minutes = elapsed_time / 60;
        let elapsed_seconds = elapsed_time % 60;
        let total_minutes = total_time / 60;
        let total_seconds = total_time % 60;

        // Print progress bar and time
        print!(
            "\r{} {:02}:{:02} / {:02}:{:02}",
            bar,
            elapsed_minutes,
            elapsed_seconds,
            total_minutes,
            total_seconds,
        );
        std::io::stdout().flush().unwrap();

        thread::sleep(std::time::Duration::from_secs(1));
        played = response.SONGTIMES.PLAYED + start_time.elapsed().as_secs() as u32;

        if played >= duration {
            break;
        }
    }

    Ok(())
}

// Main function to handle radio playback
#[tokio::main]
async fn spawn_radio() -> Result<()> {
    let (_stream, handle) = rodio::OutputStream::try_default()?;
    let sink = rodio::Sink::try_new(&handle).expect("Failed to create sink");

    let client = Client::builder().request_icy_metadata().build()?;
    let stream = HttpStream::new(client, "https://stream.gensokyoradio.net/3".parse()?).await?;

    let icy_headers = IcyHeaders::parse_from_headers(stream.headers());
    if log_enabled!(Level::Debug) {
        debug!("Icescast headers: {icy_headers:#?}\n");
        debug!("content type={:?}\n", stream.content_type());
    }

    // Buffer 10 seconds of audio
    let prefetch_bytes = icy_headers.bitrate().unwrap() / 8 * 1024 * 10;

    let reader = StreamDownload::from_stream(
        stream,
        BoundedStorageProvider::new(
            MemoryStorageProvider,
            NonZeroUsize::new(512 * 1024).unwrap(),
        ),
        Settings::default().prefetch_bytes(prefetch_bytes as u64),
    )
    .await?;

    // Wait 10 seconds for buffer
    loading_animation(Duration::from_secs(10)).await;

    if log_enabled!(Level::Debug) {
        let size = std::mem::size_of_val(&reader);
        debug!("Reader is: {}", size);
    }

    // Create a channel
    let (tx, mut rx) = mpsc::channel::<()>(10);

    // Spawn a task to handle messages from the channel
    tokio::spawn(async move {
        while let Some(_) = rx.recv().await {
            if let Err(e) = fetch_metadata().await {
                error!("Failed to fetch metadata: {}", e);
            }
        }
    });

    // Create a shared state for storing the last metadata
    let last_metadata = Arc::new(Mutex::new(String::new()));

    sink.append(
        rodio::Decoder::new(IcyMetadataReader::new(
            reader,
            icy_headers.metadata_interval(),
            {
                let last_metadata = Arc::clone(&last_metadata);
                move |metadata| {
                    let mut last = last_metadata.lock().unwrap();
                    if let Ok(parsed_metadata) = metadata {
                        let metadata_string = format!("{:?}", parsed_metadata);
                        if *last != metadata_string {
                            *last = metadata_string.clone();
                            if let Err(e) = tx.try_send(()) {
                                error!("Failed to send metadata fetch task: {}", e);
                            }
                        }
                    } else {
                        error!("Failed to parse metadata: {:?}", metadata);
                    }
                }
            },
        ))
        .context("Failed to decode audio from metadata")?,
    );

    let handle = tokio::task::spawn_blocking(move || {
        sink.sleep_until_end();
    });
    handle.await?;
    Ok(())
}

// Entry point of the program
fn main() {
    env_logger::init();
    loop {
        match spawn_radio() {
            Ok(_) => {
                error!("Failed to receive new audio data from source.");
                info!("Restarting the connection.");
            }
            Err(_) => {
                error!("Failed to create decoder for audio stream.");
                info!("Restarting the connection.");
            }
        }
    }
}