use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::Stream;
use gtunnel::args::parse_args;
use gtunnel::copy::{stream_reader_copy, stream_writer_copy};
use gtunnel::pb::{
    tunnel_server::{self, Tunnel},
    Data,
};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tonic::{transport::Server, Request, Response, Status, Streaming};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

#[tonic::async_trait]
impl Tunnel for TunnelServer {
    type TunnelStream = Pin<Box<dyn Stream<Item = Result<Data, Status>> + Send>>;

    async fn tunnel(
        &self,
        req: Request<Streaming<Data>>,
    ) -> Result<Response<Self::TunnelStream>, Status> {
        let current = self.next.fetch_add(1, Ordering::Relaxed) % self.remote_addrs.len();
        let addr = self.remote_addrs[current];
        log::debug!("proxy {:?} to {}", req.remote_addr(), addr);

        let stream = match timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(stream) => match stream {
                Ok(stream) => Ok(stream),
                Err(e) => {
                    log::error!("connect to {} err {:?}", &addr, e);
                    Err(Status::internal(e.to_string()))
                }
            },
            Err(e) => {
                log::error!("connect to {} err {:?}", &addr, e);
                Err(Status::internal(e.to_string()))
            }
        }?;

        let (reader, writer) = stream.into_split();
        tokio::spawn(async move {
            if let Err(e) = stream_writer_copy(writer, req.into_inner()).await {
                log::error!("recv stream err: {:?}", e);
            }
        });
        Ok(Response::new(Box::pin(stream_reader_copy(reader))))
    }
}

pub struct TunnelServer {
    remote_addrs: Vec<SocketAddr>,
    next: AtomicUsize,
}

#[tokio::main]
async fn main() -> io::Result<()> {
    env_logger::init();
    let cfg = parse_args("qtunnel-server").expect("invalid config");
    log::info!("{}", serde_json::to_string_pretty(&cfg).unwrap());

    let server = TunnelServer {
        remote_addrs: cfg.remote_socket_addrs(),
        next: AtomicUsize::new(0),
    };
    Server::builder()
        .add_service(tunnel_server::TunnelServer::new(server))
        .serve(cfg.local_addr.parse().unwrap())
        .await
        .unwrap();
    Ok(())
}
