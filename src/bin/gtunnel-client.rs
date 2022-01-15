macro_rules! ready {
    ($e:expr $(,)?) => {
        match $e {
            std::task::Poll::Ready(t) => t,
            std::task::Poll::Pending => return std::task::Poll::Pending,
        }
    };
}

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;
use gtunnel::args::parse_args;
use gtunnel::other;
use gtunnel::pb::{tunnel_client::TunnelClient, Data};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpListener;
use tonic::transport::Channel;
use tonic::Streaming;

#[tokio::main]
async fn main() -> io::Result<()> {
    env_logger::init();
    let config = parse_args("gtunnel-client").expect("invalid config");
    log::info!("{}", serde_json::to_string_pretty(&config).unwrap());

    let listener = TcpListener::bind(&config.local_addr).await?;
    let channel = Channel::builder(config.remote_addr.parse().unwrap())
        .connect()
        .await
        .unwrap();
    let grpc_client = TunnelClient::new(channel);

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                log::debug!("accept tcp from {:?}", addr);
                let mut grpc_client = grpc_client.clone();
                tokio::spawn(async move {
                    let (reader, writer) = stream.into_split();
                    let send_stream = stream_reader_copy(reader);
                    match grpc_client.tunnel(send_stream).await {
                        Ok(response) => {
                            let inbound = response.into_inner();
                            let recv_stream = stream_writer_copy(writer, inbound);
                            if let Err(e) = recv_stream.await {
                                log::error!("recv stream err: {:?}", e);
                            }
                        }
                        Err(err) => {
                            log::error!("grpc tunnel err: {:?}", err);
                        }
                    }
                });
            }
            Err(e) => {
                log::error!("accept fail: {:?}", e);
            }
        }
    }
}

fn stream_reader_copy(reader: OwnedReadHalf) -> StreamReaderCopy {
    StreamReaderCopy {
        reader,
        buf: vec![0u8; 2048],
    }
}

fn stream_writer_copy(writer: OwnedWriteHalf, stream: Streaming<Data>) -> StreamWriterCopy {
    StreamWriterCopy {
        writer,
        stream,
        buf: vec![],
        pos: 0,
        cap: 0,
        read_done: false,
        amt: 0,
    }
}

struct StreamWriterCopy {
    writer: OwnedWriteHalf,
    stream: Streaming<Data>,
    buf: Vec<u8>,
    pos: usize,
    read_done: bool,
    cap: usize,
    amt: u64,
}

impl Future for StreamWriterCopy {
    type Output = io::Result<u64>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let me = &mut *self;
        loop {
            // If our buffer is empty, then we need to read some data to
            // continue.
            if me.pos == me.cap && !me.read_done {
                match ready!(Pin::new(&mut me.stream).poll_next(cx)) {
                    Some(response) => {
                        let data = response.map_err(|e| other(&e.to_string()))?;
                        me.buf = data.data;
                        me.pos = 0;
                        me.cap = me.buf.len();
                    }
                    None => {
                        me.read_done = true;
                    }
                }
            }

            // If our buffer has some data, let's write it out!
            while me.pos < me.cap {
                let i = ready!(Pin::new(&mut me.writer).poll_write(cx, &me.buf[me.pos..me.cap]))?;
                if i == 0 {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "write zero byte into writer",
                    )));
                } else {
                    me.pos += i;
                    me.amt += i as u64;
                }
            }

            // If we've written all the data and we've seen EOF, flush out the
            // data and finish the transfer.
            if me.pos == me.cap && me.read_done {
                ready!(Pin::new(&mut me.writer).poll_flush(cx))?;
                return Poll::Ready(Ok(me.amt));
            }
        }
    }
}

struct StreamReaderCopy {
    reader: OwnedReadHalf,
    buf: Vec<u8>,
}

impl Stream for StreamReaderCopy {
    type Item = Data;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let me = &mut *self;
        let mut buf = ReadBuf::new(&mut me.buf[..]);
        match ready!(Pin::new(&mut me.reader).poll_read(cx, &mut buf)) {
            Ok(()) => {
                let n = buf.filled().len();
                if n == 0 {
                    return Poll::Ready(None);
                }
                return Poll::Ready(Some(Data {
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
