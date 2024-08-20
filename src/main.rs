use std::num::NonZeroUsize;

use log::{debug, error, info, log_enabled, Level};

use anyhow::{Context, Result};

use icy_metadata::{IcyHeaders, IcyMetadataReader, RequestIcyMetadata};

use stream_download::http::reqwest::Client;
use stream_download::http::HttpStream;
use stream_download::storage::bounded::BoundedStorageProvider;
use stream_download::storage::memory::MemoryStorageProvider;
use stream_download::{Settings, StreamDownload};

#[tokio::main]
async fn spawn_radio() -> Result<()> {
    let (_stream, handle) = rodio::OutputStream::try_default()?;
    let sink = rodio::Sink::try_new(&handle).expect("Failed to create sink");

    let client = Client::builder().request_icy_metadata().build()?;

    let stream = HttpStream::new(client, "https://stream.gensokyoradio.net/1".parse()?).await?;

    let icy_headers = IcyHeaders::parse_from_headers(stream.headers());
    if log_enabled!(Level::Debug) {
        debug!("Icescast headers: {icy_headers:#?}\n");
        debug!("content type={:?}\n", stream.content_type());
    }

    // buffer 20 seconds of audio
    // bitrate (in kilobits) / bits per byte * bytes per kilobyte * 20 seconds
    let prefetch_bytes = icy_headers.bitrate().unwrap() / 8 * 1024 * 20;

    let reader = StreamDownload::from_stream(
        stream,
        BoundedStorageProvider::new(
            MemoryStorageProvider,
            NonZeroUsize::new(512 * 1024).unwrap(),
        ),
        Settings::default().prefetch_bytes(prefetch_bytes as u64),
    )
    .await?;

    if log_enabled!(Level::Debug) {
        let size = std::mem::size_of_val(&reader);
        debug!("Reader is: {}", size);
    }

    sink.append(
        rodio::Decoder::new(IcyMetadataReader::new(
            reader,
            // Since we requested icy metadata, the metadata interval header should be present in the
            // response. This will allow us to parse the metadata within the stream
            icy_headers.metadata_interval(),
            |metadata| println!("{metadata:#?}\n"),
        ))
        .context("Failed to decode audio from metadata")?,
    );

    let handle = tokio::task::spawn_blocking(move || {
        sink.sleep_until_end();
    });
    handle.await?;
    Ok(())
}

fn main() {
    env_logger::init();
    loop {
        match spawn_radio() {
            Ok(_) => {
                error!("Failed to recieve new Audio data from source.");
                info!("Restarting the connection.");
            }
            Err(_) => {
                error!("Failed to create Decoder for audio stream.");
                info!("Restarting the connection.");
            }
        }
    }
}
