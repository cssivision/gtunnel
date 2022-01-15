use std::io;

use futures_util::StreamExt;
use gtunnel::args::parse_args;
use gtunnel::copy::{stream_reader_copy, stream_writer_copy};
use gtunnel::pb::tunnel_client::TunnelClient;
use tokio::net::{TcpListener, TcpStream};
use tonic::transport::Channel;

const DEFAULT_CONN_WINDOW: u32 = 1024 * 1024 * 8; // 8mb
const DEFAULT_STREAM_WINDOW: u32 = 1024 * 1024; // 1mb

#[tokio::main]
async fn main() -> io::Result<()> {
    env_logger::init();
    let config = parse_args("gtunnel-client").expect("invalid config");
    log::info!("{}", serde_json::to_string_pretty(&config).unwrap());

    let listener = TcpListener::bind(&config.local_addr).await?;
    let channel = Channel::builder(config.remote_addr.parse().unwrap())
        .initial_connection_window_size(DEFAULT_CONN_WINDOW)
        .initial_stream_window_size(DEFAULT_STREAM_WINDOW)
        .connect()
        .await
        .unwrap();
    let client = TunnelClient::new(channel);

    loop {
        let (stream, addr) = listener.accept().await?;
        log::debug!("accept tcp from {:?}", addr);
        proxy(stream, client.clone());
    }
}

fn proxy(stream: TcpStream, mut client: TunnelClient<Channel>) {
    tokio::spawn(async move {
        let (reader, writer) = stream.into_split();
        let mut send_stream = stream_reader_copy(reader);
        let req = async_stream::stream! {
            while let Some(item) = send_stream.next().await {
                match item {
                    Ok(v) => {
                        yield v;
                    }
                    Err(_) => {
                        break;
                    }
                }
            }
        };

        match client.tunnel(req).await {
            Ok(response) => {
                if let Err(e) = stream_writer_copy(writer, response.into_inner()).await {
                    log::error!("recv stream err: {:?}", e);
                }
            }
            Err(err) => {
                log::error!("grpc tunnel err: {:?}", err);
            }
        }
    });
}
