use std::num::NonZeroUsize;

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

    let stream =
        HttpStream::new(client, "https://stream.gensokyoradio.net/3".parse()?).await?;

    let icy_headers = IcyHeaders::parse_from_headers(stream.headers());
    println!("Icescast headers: {icy_headers:#?}\n");
    println!("content type={:?}\n", stream.content_type());

    // buffer 5 seconds of audio
    // bitrate (in kilobits) / bits per byte * bytes per kilobyte * 5 seconds
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

    sink.append(rodio::Decoder::new(IcyMetadataReader::new(
        reader,
        // Since we requested icy metadata, the metadata interval header should be present in the
        // response. This will allow us to parse the metadata within the stream
        icy_headers.metadata_interval(),
        |metadata| println!("{metadata:#?}\n"),
    )).context("Failed to decode audio from metadata")?);

    let handle = tokio::task::spawn_blocking(move || {
        sink.sleep_until_end();
    });
    handle.await?;
    Ok(())
}

fn main() {
    loop {
        match spawn_radio() {
            Ok(_) => {
                println!("Failed to recieve new Audio data from source.\nRestarting the connection.");
            },
            Err(_) => {
                println!("Failed to create Decoder for audio stream.\nRestarting the connection.");
            },
        }
    }
}
