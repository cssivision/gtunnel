use std::io;
use std::time::Duration;

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
        .connect_timeout(Duration::from_secs(3))
        .connect_lazy();
    let client = TunnelClient::new(channel);

    loop {
        let (stream, addr) = listener.accept().await?;
        log::debug!("accept tcp from {:?}", addr);
        let client = client.clone();
        tokio::spawn(async move {
            proxy(stream, client).await;
        });
    }
}

async fn proxy(stream: TcpStream, mut client: TunnelClient<Channel>) {
    let (reader, writer) = stream.into_split();
    let send_stream = stream_reader_copy(reader).filter_map(|v| async { v.ok() });

    match client.tunnel(send_stream).await {
        Ok(response) => {
            if let Err(e) = stream_writer_copy(writer, response.into_inner()).await {
                log::error!("recv stream err: {:?}", e);
            }
        }
        Err(err) => {
            log::error!("grpc tunnel err: {:?}", err);
        }
    }
}
