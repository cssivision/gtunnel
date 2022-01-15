use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tonic::{Status, Streaming};

use crate::other;
use crate::pb::Data;

pub fn stream_writer_copy(writer: OwnedWriteHalf, stream: Streaming<Data>) -> StreamWriterCopy {
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

pub struct StreamWriterCopy {
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

pub fn stream_reader_copy(reader: OwnedReadHalf) -> StreamReaderCopy {
    StreamReaderCopy {
        reader,
        buf: vec![0u8; 2048],
    }
}

pub struct StreamReaderCopy {
    reader: OwnedReadHalf,
    buf: Vec<u8>,
}

impl Stream for StreamReaderCopy {
    type Item = Result<Data, Status>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let me = &mut *self;
        let mut buf = ReadBuf::new(&mut me.buf[..]);
        match ready!(Pin::new(&mut me.reader).poll_read(cx, &mut buf)) {
            Ok(()) => {
                let n = buf.filled().len();
                if n == 0 {
                    return Poll::Ready(None);
                }
                return Poll::Ready(Some(Ok(Data {
                    data: me.buf[..n].to_vec(),
                })));
            }
            Err(err) => {
                log::error!("stream poll_read err: {:?}", err);
                return Poll::Ready(Some(Err(Status::internal(format!(
                    "stream poll_read err: {:?}",
                    err
                )))));
            }
        }
    }
}
