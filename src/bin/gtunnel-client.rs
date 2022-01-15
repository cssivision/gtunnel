macro_rules! ready {
    ($e:expr $(,)?) => {
        match $e {
            std::task::Poll::Ready(t) => t,
            std::task::Poll::Pending => return std::task::Poll::Pending,
        }
    };
}

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;
use gtunnel::args::parse_args;
use gtunnel::other;
use tokio::io::{AsyncRead, ReadBuf};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
pub mod pb {
    tonic::include_proto!("tunnel");
}
use pb::greeter_client::GreeterClient;

#[tokio::main]
async fn main() -> io::Result<()> {
    env_logger::init();
    let config = parse_args("mtunnel-client").expect("invalid config");
    log::info!("{}", serde_json::to_string_pretty(&config).unwrap());

    let listener = TcpListener::bind(&config.local_addr).await?;
    let mut grpc_client = GreeterClient::connect(config.remote_addr)
        .await
        .map_err(|e| other(&e.to_string()))?;

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                log::debug!("accept tcp from {:?}", addr);
                let (send_stream, recv_stream) = split(stream);
                match grpc_client.tunnel(send_stream).await {
                    Ok(rsp_stream) => {
                        tokio::spawn(async move {});
                    }
                    Err(err) => {
                        log::error!("grpc tunnel err: {:?}", err);
                    }
                }
            }
            Err(e) => {
                log::error!("accept fail: {:?}", e);
            }
        }
    }
}

fn split(stream: TcpStream) -> (SendStream, RecvStream) {
    let (reader, writer) = stream.into_split();
    (
        SendStream {
            reader,
            buf: vec![0u8; 2048],
        },
        RecvStream { writer },
    )
}

struct RecvStream {
    writer: OwnedWriteHalf,
}

struct SendStream {
    reader: OwnedReadHalf,
    buf: Vec<u8>,
}

impl Stream for SendStream {
    type Item = pb::Data;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let me = &mut *self;
        let mut buf = ReadBuf::new(&mut me.buf[..]);
        match ready!(Pin::new(&mut me.reader).poll_read(cx, &mut buf)) {
            Ok(()) => {
                let n = buf.filled().len();
                if n == 0 {
                    return Poll::Ready(None);
                }
                return Poll::Ready(Some(pb::Data {
                    data: me.buf[..n].to_vec(),
                }));
            }
            Err(err) => {
                log::error!("stream poll_read err: {:?}", err);
                return Poll::Ready(None);
            }
        }
    }
}
