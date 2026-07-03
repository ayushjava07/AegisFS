use std::pin::Pin;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::core::error::{AegisError, AegisResult};
use crate::core::traits::*;

pub struct AsyncStreamReader {
    inner: Pin<Box<dyn AsyncRead + Send>>,
}

impl AsyncStreamReader {
    pub fn new<R: AsyncRead + Send + 'static>(reader: R) -> Self {
        Self {
            inner: Box::pin(reader),
        }
    }
}

impl StreamingReader for AsyncStreamReader {
    fn read<'a>(&'a mut self, buf: &'a mut [u8]) -> BoxFuture<'a, AegisResult<usize>> {
        let inner = &mut self.inner;
        Box::pin(async move { inner.as_mut().read(buf).await.map_err(AegisError::Io) })
    }

    fn read_exact<'a>(&'a mut self, buf: &'a mut [u8]) -> BoxFuture<'a, AegisResult<()>> {
        let inner = &mut self.inner;
        Box::pin(async move {
            let mut offset = 0;
            while offset < buf.len() {
                let n = inner
                    .as_mut()
                    .read(&mut buf[offset..])
                    .await
                    .map_err(AegisError::Io)?;
                if n == 0 {
                    return Err(AegisError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "unexpected EOF",
                    )));
                }
                offset += n;
            }
            Ok(())
        })
    }

    fn close(&mut self) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move { Ok(()) })
    }
}

pub struct AsyncStreamWriter {
    inner: Pin<Box<dyn AsyncWrite + Send>>,
}

impl AsyncStreamWriter {
    pub fn new<W: AsyncWrite + Send + 'static>(writer: W) -> Self {
        Self {
            inner: Box::pin(writer),
        }
    }
}

impl StreamingWriter for AsyncStreamWriter {
    fn write<'a>(&'a mut self, buf: &'a [u8]) -> BoxFuture<'a, AegisResult<usize>> {
        let inner = &mut self.inner;
        Box::pin(async move { inner.as_mut().write(buf).await.map_err(AegisError::Io) })
    }

    fn flush(&mut self) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move { self.inner.as_mut().flush().await.map_err(AegisError::Io) })
    }

    fn close(&mut self) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move { self.inner.as_mut().shutdown().await.map_err(AegisError::Io) })
    }
}

pub struct AsyncFileAdapter {
    file: tokio::fs::File,
}

impl AsyncFileAdapter {
    pub async fn open(path: &str) -> AegisResult<Self> {
        let file = tokio::fs::File::open(path).await.map_err(AegisError::Io)?;
        Ok(Self { file })
    }

    pub async fn create(path: &str) -> AegisResult<Self> {
        let file = tokio::fs::File::create(path)
            .await
            .map_err(AegisError::Io)?;
        Ok(Self { file })
    }

    pub fn from_file(file: tokio::fs::File) -> Self {
        Self { file }
    }
}

impl StreamingReader for AsyncFileAdapter {
    fn read<'a>(&'a mut self, buf: &'a mut [u8]) -> BoxFuture<'a, AegisResult<usize>> {
        let file = &mut self.file;
        Box::pin(async move { file.read(buf).await.map_err(AegisError::Io) })
    }

    fn read_exact<'a>(&'a mut self, buf: &'a mut [u8]) -> BoxFuture<'a, AegisResult<()>> {
        let file = &mut self.file;
        Box::pin(async move {
            let mut offset = 0;
            while offset < buf.len() {
                let n = file
                    .read(&mut buf[offset..])
                    .await
                    .map_err(AegisError::Io)?;
                if n == 0 {
                    return Err(AegisError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "unexpected EOF",
                    )));
                }
                offset += n;
            }
            Ok(())
        })
    }

    fn close(&mut self) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move { self.file.shutdown().await.map_err(AegisError::Io) })
    }
}

impl StreamingWriter for AsyncFileAdapter {
    fn write<'a>(&'a mut self, buf: &'a [u8]) -> BoxFuture<'a, AegisResult<usize>> {
        let file = &mut self.file;
        Box::pin(async move { file.write(buf).await.map_err(AegisError::Io) })
    }

    fn flush(&mut self) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move { self.file.flush().await.map_err(AegisError::Io) })
    }

    fn close(&mut self) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move { self.file.shutdown().await.map_err(AegisError::Io) })
    }
}

pub struct PipeStream {
    inner: tokio::io::DuplexStream,
}

impl PipeStream {
    pub fn pair() -> (Self, Self) {
        let (a, b) = tokio::io::duplex(64 * 1024);
        (Self { inner: a }, Self { inner: b })
    }
}

impl StreamingReader for PipeStream {
    fn read<'a>(&'a mut self, buf: &'a mut [u8]) -> BoxFuture<'a, AegisResult<usize>> {
        let inner = &mut self.inner;
        Box::pin(async move { inner.read(buf).await.map_err(AegisError::Io) })
    }

    fn read_exact<'a>(&'a mut self, buf: &'a mut [u8]) -> BoxFuture<'a, AegisResult<()>> {
        let inner = &mut self.inner;
        Box::pin(async move {
            let mut offset = 0;
            while offset < buf.len() {
                let n = inner
                    .read(&mut buf[offset..])
                    .await
                    .map_err(AegisError::Io)?;
                if n == 0 {
                    return Err(AegisError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "unexpected EOF",
                    )));
                }
                offset += n;
            }
            Ok(())
        })
    }

    fn close(&mut self) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move { self.inner.shutdown().await.map_err(AegisError::Io) })
    }
}

impl StreamingWriter for PipeStream {
    fn write<'a>(&'a mut self, buf: &'a [u8]) -> BoxFuture<'a, AegisResult<usize>> {
        let inner = &mut self.inner;
        Box::pin(async move { inner.write(buf).await.map_err(AegisError::Io) })
    }

    fn flush(&mut self) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move { self.inner.flush().await.map_err(AegisError::Io) })
    }

    fn close(&mut self) -> BoxFuture<'_, AegisResult<()>> {
        Box::pin(async move { self.inner.shutdown().await.map_err(AegisError::Io) })
    }
}

pub struct BufferedStream {
    inner: Box<dyn StreamingReader>,
    buffer: Vec<u8>,
    pos: usize,
    cap: usize,
}

impl BufferedStream {
    pub fn new(inner: Box<dyn StreamingReader>) -> Self {
        Self::with_capacity(inner, 8192)
    }

    pub fn with_capacity(inner: Box<dyn StreamingReader>, buf_size: usize) -> Self {
        Self {
            inner,
            buffer: vec![0u8; buf_size],
            pos: 0,
            cap: 0,
        }
    }
}

impl StreamingReader for BufferedStream {
    fn read<'a>(&'a mut self, buf: &'a mut [u8]) -> BoxFuture<'a, AegisResult<usize>> {
        Box::pin(async move {
            if self.pos >= self.cap {
                self.cap = self.inner.read(&mut self.buffer).await?;
                self.pos = 0;
                if self.cap == 0 {
                    return Ok(0);
                }
            }
            let avail = self.cap - self.pos;
            let to_copy = buf.len().min(avail);
            buf[..to_copy].copy_from_slice(&self.buffer[self.pos..self.pos + to_copy]);
            self.pos += to_copy;
            Ok(to_copy)
        })
    }

    fn read_exact<'a>(&'a mut self, buf: &'a mut [u8]) -> BoxFuture<'a, AegisResult<()>> {
        Box::pin(async move {
            let mut offset = 0;
            while offset < buf.len() {
                if self.pos >= self.cap {
                    self.cap = self.inner.read(&mut self.buffer).await?;
                    self.pos = 0;
                    if self.cap == 0 {
                        return Err(AegisError::Io(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "unexpected EOF",
                        )));
                    }
                }
                let avail = self.cap - self.pos;
                let to_copy = (buf.len() - offset).min(avail);
                buf[offset..offset + to_copy]
                    .copy_from_slice(&self.buffer[self.pos..self.pos + to_copy]);
                self.pos += to_copy;
                offset += to_copy;
            }
            Ok(())
        })
    }

    fn close(&mut self) -> BoxFuture<'_, AegisResult<()>> {
        self.inner.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_async_stream_reader_writer_roundtrip() {
        let (mut reader, mut writer) = PipeStream::pair();

        let write_data = b"hello world";
        writer.write(write_data).await.unwrap();
        writer.flush().await.unwrap();
        drop(writer);

        let mut buf = vec![0u8; 11];
        reader.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, write_data);
    }

    #[tokio::test]
    async fn test_pipe_stream_pair() {
        let (mut a, mut b) = PipeStream::pair();

        a.write(b"ping").await.unwrap();
        a.flush().await.unwrap();

        let mut buf = vec![0u8; 4];
        b.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");

        b.write(b"pong").await.unwrap();
        b.flush().await.unwrap();

        let mut buf2 = vec![0u8; 4];
        a.read_exact(&mut buf2).await.unwrap();
        assert_eq!(&buf2, b"pong");
    }

    #[tokio::test]
    async fn test_buffered_stream_buffering() {
        let (raw_reader, mut writer) = PipeStream::pair();
        let data = vec![0xABu8; 100];
        writer.write(&data).await.unwrap();
        writer.flush().await.unwrap();
        drop(writer);

        let reader: Box<dyn StreamingReader> = Box::new(raw_reader);
        let mut buffered = BufferedStream::with_capacity(reader, 32);

        let mut buf = vec![0u8; 100];
        buffered.read_exact(&mut buf).await.unwrap();
        assert_eq!(buf, data);
    }

    #[tokio::test]
    async fn test_buffered_stream_partial_reads() {
        let (raw_reader, mut writer) = PipeStream::pair();
        let data = vec![0x42u8; 64];
        writer.write(&data).await.unwrap();
        writer.flush().await.unwrap();
        drop(writer);

        let reader: Box<dyn StreamingReader> = Box::new(raw_reader);
        let mut buffered = BufferedStream::with_capacity(reader, 32);

        let mut first = vec![0u8; 10];
        let n = buffered.read(&mut first).await.unwrap();
        assert_eq!(n, 10);
        assert_eq!(&first, &[0x42u8; 10]);

        let mut second = vec![0u8; 54];
        buffered.read_exact(&mut second).await.unwrap();
        assert_eq!(&second, &[0x42u8; 54]);
    }

    #[tokio::test]
    async fn test_stream_close() {
        let (mut reader, mut writer) = PipeStream::pair();
        writer.write(b"data").await.unwrap();
        StreamingWriter::close(&mut writer).await.unwrap();

        let mut buf = vec![0u8; 4];
        reader.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"data");

        let n = reader.read(&mut buf).await.unwrap();
        assert_eq!(n, 0);
    }
}
