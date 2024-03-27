use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use fastanvil::Error;
use flate2::read::ZlibEncoder;
use flate2::Compression;
use num_enum_derive::TryFromPrimitive;
use std::io;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub(crate) const SECTOR_SIZE: usize = 4096;
pub(crate) const REGION_HEADER_SIZE: usize = 2 * SECTOR_SIZE;
pub(crate) const CHUNK_HEADER_SIZE: usize = 5;

#[derive(Clone)]
pub struct Region<S> {
    stream: S,
    offsets: Vec<u64>,
}

impl<S> Region<S>
where
    S: AsyncReadExt + AsyncSeekExt + Unpin,
{
    pub async fn from_async_stream(stream: S) -> fastanvil::Result<Self> {
        let mut tmp = Self {
            stream,
            offsets: vec![],
        };

        let mut max_offset = 0;
        let mut max_offsets_sector_count = 0;

        for z in 0..32 {
            for x in 0..32 {
                let Some(loc) = tmp.async_location(x, z).await? else {
                    continue;
                };

                tmp.offsets.push(loc.offset);
                if loc.offset > max_offset {
                    max_offset = loc.offset;
                    max_offsets_sector_count = loc.sectors;
                }
            }
        }

        tmp.offsets.sort_unstable();
        tmp.offsets.push(max_offset + max_offsets_sector_count);
        Ok(tmp)
    }

    pub(crate) async fn async_location(
        &mut self,
        x: usize,
        z: usize,
    ) -> fastanvil::Result<Option<ChunkLocation>> {
        if x >= 32 || z >= 32 {
            return Err(Error::InvalidOffset(x as isize, z as isize));
        }

        self.stream.seek(SeekFrom::Start(header_pos(x, z))).await?;

        let mut buf = [0u8; 4];
        self.stream.read_exact(&mut buf[..]).await?;

        let mut offset = 0u64;
        offset |= (buf[0] as u64) << 16;
        offset |= (buf[1] as u64) << 8;
        offset |= buf[2] as u64;
        let sectors = buf[3] as u64;

        Ok((offset != 0 || sectors != 0).then_some(ChunkLocation { offset, sectors }))
    }

    pub async fn async_read_chunk(
        &mut self,
        x: usize,
        z: usize,
    ) -> fastanvil::Result<Option<Vec<u8>>> {
        let scheme = self.async_compression_scheme(x, z).await?;
        match scheme {
            None => None,
            Some(scheme) => Some(match scheme {
                fastanvil::CompressionScheme::Zlib => {
                    let mut decoder =
                        async_compression::tokio::write::ZlibDecoder::new(Cursor::new(vec![]));
                    self.async_read_compressed_chunk(x, z, &mut decoder).await?;
                    Ok(decoder.into_inner().into_inner())
                }
                fastanvil::CompressionScheme::Gzip => {
                    let mut decoder =
                        async_compression::tokio::write::GzipDecoder::new(Cursor::new(vec![]));
                    self.async_read_compressed_chunk(x, z, &mut decoder).await?;
                    Ok(decoder.into_inner().into_inner())
                }
                fastanvil::CompressionScheme::Uncompressed => {
                    let mut buf = vec![];
                    self.async_read_compressed_chunk(x, z, &mut buf).await?;
                    Ok(buf)
                }
            }),
        }
        .transpose()
    }

    async fn async_read_compressed_chunk<W>(
        &mut self,
        x: usize,
        z: usize,
        writer: &mut W,
    ) -> fastanvil::Result<bool>
    where
        W: AsyncWriteExt + Unpin,
    {
        let Some(loc) = self.async_location(x, z).await? else {
            return Ok(false);
        };

        self.stream
            .seek(SeekFrom::Start(loc.offset * SECTOR_SIZE as u64))
            .await?;

        let mut buf = [0u8; 5];
        self.stream.read_exact(&mut buf).await?;
        let metadata = ChunkMeta::new(&buf)?;

        let mut adapted = (&mut self.stream).take(metadata.compressed_len as u64);

        tokio::io::copy(&mut adapted, writer).await?;

        Ok(true)
    }

    async fn async_compression_scheme(
        &mut self,
        x: usize,
        z: usize,
    ) -> fastanvil::Result<Option<fastanvil::CompressionScheme>> {
        if x >= 32 || z >= 32 {
            return Err(Error::InvalidOffset(x as isize, z as isize));
        }

        let Some(loc) = self.async_location(x, z).await? else {
            return Ok(None);
        };

        self.stream
            .seek(SeekFrom::Start(loc.offset * SECTOR_SIZE as u64))
            .await?;

        let mut buf = [0u8; 5];
        self.stream.read_exact(&mut buf).await?;
        let metadata = ChunkMeta::new(&buf)?;

        Ok(Some(metadata.compression_scheme))
    }

    async fn async_chunk_meta(
        &self,
        compressed_chunk_size: u32,
        scheme: fastanvil::CompressionScheme,
    ) -> [u8; 5] {
        let mut buf = [0u8; 5];
        let mut c = Cursor::new(buf.as_mut_slice());

        WriteBytesExt::write_u32::<BigEndian>(&mut c, compressed_chunk_size + 1).unwrap();
        WriteBytesExt::write_u8(
            &mut c,
            match scheme {
                fastanvil::CompressionScheme::Gzip => 1,
                fastanvil::CompressionScheme::Zlib => 2,
                fastanvil::CompressionScheme::Uncompressed => 3,
            },
        )
        .unwrap();

        buf
    }

    pub async fn iter(&mut self) -> RegionIter<'_, S> {
        RegionIter::async_new(self).await
    }
}

impl<S> Region<S>
where
    S: AsyncReadExt + AsyncWriteExt + AsyncSeekExt + Unpin,
{
    pub async fn async_new(mut stream: S) -> fastanvil::Result<Self> {
        stream.rewind().await?;
        stream.write_all(&[0; REGION_HEADER_SIZE]).await?;

        Ok(Self {
            stream,
            offsets: vec![2],
        })
    }

    pub async fn async_write_chunk(
        &mut self,
        x: usize,
        z: usize,
        uncompressed_chunk: &[u8],
    ) -> fastanvil::Result<()> {
        let mut buf = vec![];
        let mut enc = ZlibEncoder::new(uncompressed_chunk, Compression::fast());
        enc.read_to_end(&mut buf)?;
        self.async_write_compressed_chunk(x, z, fastanvil::CompressionScheme::Zlib, &buf)
            .await
    }

    pub async fn async_write_compressed_chunk(
        &mut self,
        x: usize,
        z: usize,
        scheme: fastanvil::CompressionScheme,
        compressed_chunk: &[u8],
    ) -> fastanvil::Result<()> {
        let loc = self.async_location(x, z).await?;
        let required_sectors =
            unstable_div_ceil(CHUNK_HEADER_SIZE + compressed_chunk.len(), SECTOR_SIZE);
        if let Some(loc) = loc {
            let i = self.offsets.binary_search(&loc.offset).unwrap();
            let start_offset = self.offsets[i];
            let end_offset = self.offsets[i + 1];
            let available_sectors = (end_offset - start_offset) as usize;

            if required_sectors <= available_sectors {
                self.async_set_chunk(start_offset, scheme, compressed_chunk)
                    .await?;
                self.async_set_header(x, z, start_offset, required_sectors)
                    .await?;
            } else {
                self.offsets.remove(i);
                let offset = *self.offsets.last().unwrap();
                self.offsets.push(offset + required_sectors as u64);
                self.async_set_chunk(offset, scheme, compressed_chunk)
                    .await?;
                self.async_pad().await?;
                self.async_set_header(x, z, offset, required_sectors)
                    .await?;
            }
        } else {
            let offset = *self.offsets.last().expect("offset should always exist");
            self.offsets.push(offset + required_sectors as u64);
            self.async_set_chunk(offset, scheme, compressed_chunk)
                .await?;
            self.async_pad().await?;
            self.async_set_header(x, z, offset, required_sectors)
                .await?;
        }

        Ok(())
    }

    async fn async_set_chunk(
        &mut self,
        offset: u64,
        scheme: fastanvil::CompressionScheme,
        chunk: &[u8],
    ) -> fastanvil::Result<()> {
        self.stream
            .seek(SeekFrom::Start(offset * SECTOR_SIZE as u64))
            .await?;

        self.stream
            .write_all(&self.async_chunk_meta(chunk.len() as u32, scheme).await)
            .await?;

        self.stream.write_all(chunk).await?;
        Ok(())
    }

    async fn async_pad(&mut self) -> fastanvil::Result<()> {
        let current_end = async_unstable_stream_len(&mut self.stream).await? as usize;
        let padded_end = unstable_div_ceil(current_end, SECTOR_SIZE) * SECTOR_SIZE;
        let pad_len = padded_end - current_end;
        self.stream.write_all(&vec![0; pad_len]).await?;
        Ok(())
    }

    async fn async_set_header(
        &mut self,
        x: usize,
        z: usize,
        offset: u64,
        new_sector_count: usize,
    ) -> fastanvil::Result<()> {
        if new_sector_count > 255 {
            return Err(Error::ChunkTooLarge);
        }

        let mut buf = [0u8; 4];
        buf[0] = ((offset & 0xFF0000) >> 16) as u8;
        buf[1] = ((offset & 0x00FF00) >> 8) as u8;
        buf[2] = (offset & 0x0000FF) as u8;
        buf[3] = new_sector_count as u8;

        self.stream.seek(SeekFrom::Start(header_pos(x, z))).await?;
        self.stream.write_all(&buf).await?;
        Ok(())
    }
}

impl<S> Region<S>
where
    S: Read + Seek,
{
    pub fn from_stream(stream: S) -> fastanvil::Result<Self> {
        let mut tmp = Self {
            stream,
            offsets: vec![],
        };

        let mut max_offset = 0;
        let mut max_offsets_sector_count = 0;

        for z in 0..32 {
            for x in 0..32 {
                let Some(loc) = tmp.location(x, z)? else {
                    continue;
                };

                tmp.offsets.push(loc.offset);
                if loc.offset > max_offset {
                    max_offset = loc.offset;
                    max_offsets_sector_count = loc.sectors;
                }
            }
        }

        tmp.offsets.sort_unstable();

        // we add an offset representing the end of sectors that are in use.
        tmp.offsets.push(max_offset + max_offsets_sector_count);
        Ok(tmp)
    }

    pub fn read_chunk(&mut self, x: usize, z: usize) -> fastanvil::Result<Option<Vec<u8>>> {
        self.compression_scheme(x, z)?
            .map(|scheme| match scheme {
                fastanvil::CompressionScheme::Zlib => {
                    let mut decoder = flate2::write::ZlibDecoder::new(vec![]);
                    self.read_compressed_chunk(x, z, &mut decoder)?;
                    Ok(decoder.finish()?)
                }
                fastanvil::CompressionScheme::Gzip => {
                    let mut decoder = flate2::write::GzDecoder::new(vec![]);
                    self.read_compressed_chunk(x, z, &mut decoder)?;
                    Ok(decoder.finish()?)
                }
                fastanvil::CompressionScheme::Uncompressed => {
                    let mut buf = vec![];
                    self.read_compressed_chunk(x, z, &mut buf)?;
                    Ok(buf)
                }
            })
            .transpose()
    }

    pub(crate) fn location(
        &mut self,
        x: usize,
        z: usize,
    ) -> fastanvil::Result<Option<ChunkLocation>> {
        if x >= 32 || z >= 32 {
            return Err(Error::InvalidOffset(x as isize, z as isize));
        }

        self.stream.seek(SeekFrom::Start(header_pos(x, z)))?;

        let mut buf = [0u8; 4];
        self.stream.read_exact(&mut buf[..])?;

        let mut offset = 0u64;
        offset |= (buf[0] as u64) << 16;
        offset |= (buf[1] as u64) << 8;
        offset |= buf[2] as u64;
        let sectors = buf[3] as u64;

        Ok((offset != 0 || sectors != 0).then_some(ChunkLocation { offset, sectors }))
    }

    fn read_compressed_chunk(
        &mut self,
        x: usize,
        z: usize,
        writer: &mut dyn Write,
    ) -> fastanvil::Result<bool> {
        let Some(loc) = self.location(x, z)? else {
            return Ok(false);
        };

        self.stream
            .seek(SeekFrom::Start(loc.offset * SECTOR_SIZE as u64))?;

        let mut buf = [0u8; 5];
        self.stream.read_exact(&mut buf)?;
        let metadata = ChunkMeta::new(&buf)?;

        let mut adapted = (&mut self.stream).take(metadata.compressed_len as u64);

        io::copy(&mut adapted, writer)?;

        Ok(true)
    }

    pub fn into_inner(mut self) -> io::Result<S> {
        self.offsets.pop().unwrap();
        let Some(offset) = self.offsets.pop() else {
            self.stream
                .seek(SeekFrom::Start(REGION_HEADER_SIZE as u64))?;
            return Ok(self.stream);
        };
        self.stream
            .seek(SeekFrom::Start(offset * SECTOR_SIZE as u64))?;
        let chunk_length = self.stream.read_u32::<BigEndian>()? + 4;
        let logical_end = unstable_div_ceil(
            offset as usize * SECTOR_SIZE + chunk_length as usize,
            SECTOR_SIZE,
        ) * SECTOR_SIZE;

        self.stream.seek(SeekFrom::Start(logical_end as u64))?;
        Ok(self.stream)
    }

    fn compression_scheme(
        &mut self,
        x: usize,
        z: usize,
    ) -> fastanvil::Result<Option<fastanvil::CompressionScheme>> {
        if x >= 32 || z >= 32 {
            return Err(Error::InvalidOffset(x as isize, z as isize));
        }

        let Some(loc) = self.location(x, z)? else {
            return Ok(None);
        };

        self.stream
            .seek(SeekFrom::Start(loc.offset * SECTOR_SIZE as u64))?;

        let mut buf = [0u8; 5];
        self.stream.read_exact(&mut buf)?;
        let metadata = ChunkMeta::new(&buf)?;

        Ok(Some(metadata.compression_scheme))
    }

    fn chunk_meta(
        &self,
        compressed_chunk_size: u32,
        scheme: fastanvil::CompressionScheme,
    ) -> [u8; 5] {
        let mut buf = [0u8; 5];
        let mut c = Cursor::new(buf.as_mut_slice());

        WriteBytesExt::write_u32::<BigEndian>(&mut c, compressed_chunk_size + 1).unwrap();
        WriteBytesExt::write_u8(
            &mut c,
            match scheme {
                fastanvil::CompressionScheme::Gzip => 1,
                fastanvil::CompressionScheme::Zlib => 2,
                fastanvil::CompressionScheme::Uncompressed => 3,
            },
        )
        .unwrap();

        buf
    }
}

impl<S> Region<S>
where
    S: Read + Write + Seek,
{
    pub fn new(mut stream: S) -> fastanvil::Result<Self> {
        stream.rewind()?;
        stream.write_all(&[0; REGION_HEADER_SIZE])?;

        Ok(Self {
            stream,
            offsets: vec![2],
        })
    }

    pub fn write_chunk(
        &mut self,
        x: usize,
        z: usize,
        uncompressed_chunk: &[u8],
    ) -> fastanvil::Result<()> {
        let mut buf = vec![];
        let mut enc = ZlibEncoder::new(uncompressed_chunk, Compression::fast());
        enc.read_to_end(&mut buf)?;
        self.write_compressed_chunk(x, z, fastanvil::CompressionScheme::Zlib, &buf)
    }

    pub fn write_compressed_chunk(
        &mut self,
        x: usize,
        z: usize,
        scheme: fastanvil::CompressionScheme,
        compressed_chunk: &[u8],
    ) -> fastanvil::Result<()> {
        let loc = self.location(x, z)?;
        let required_sectors =
            unstable_div_ceil(CHUNK_HEADER_SIZE + compressed_chunk.len(), SECTOR_SIZE);

        if let Some(loc) = loc {
            // chunk already exists in the region file, need to update it.
            let i = self.offsets.binary_search(&loc.offset).unwrap();
            let start_offset = self.offsets[i];
            let end_offset = self.offsets[i + 1];
            let available_sectors = (end_offset - start_offset) as usize;

            if required_sectors <= available_sectors {
                // we fit in the current gap in the file.
                self.set_chunk(start_offset, scheme, compressed_chunk)?;
                self.set_header(x, z, start_offset, required_sectors)?;
            } else {
                // we do not fit in the current gap, need to find a new home for
                // this chunk.
                self.offsets.remove(i); // this chunk will no longer be here.
                let offset = *self.offsets.last().unwrap();

                // add a new offset representing the new 'end' of the current region file.
                self.offsets.push(offset + required_sectors as u64);
                self.set_chunk(offset, scheme, compressed_chunk)?;
                self.pad()?;
                self.set_header(x, z, offset, required_sectors)?;
            }
        } else {
            // chunk does not exist in the region yet.
            let offset = *self.offsets.last().expect("offset should always exist");

            // add a new offset representing the new 'end' of the current region file.
            self.offsets.push(offset + required_sectors as u64);
            self.set_chunk(offset, scheme, compressed_chunk)?;
            self.pad()?;
            self.set_header(x, z, offset, required_sectors)?;
        }

        Ok(())
    }

    pub fn remove_chunk(&mut self, x: usize, z: usize) -> fastanvil::Result<()> {
        let Some(loc) = self.location(x, z)? else {
            return Ok(());
        };

        // zero the region header for the chunk
        self.set_header(x, z, 0, 0)?;

        // remove the offset of the chunk
        let i = self.offsets.binary_search(&loc.offset).unwrap();
        self.offsets.remove(i);

        Ok(())
    }

    fn set_chunk(
        &mut self,
        offset: u64,
        scheme: fastanvil::CompressionScheme,
        chunk: &[u8],
    ) -> fastanvil::Result<()> {
        self.stream
            .seek(SeekFrom::Start(offset * SECTOR_SIZE as u64))?;

        self.stream.write_all(&self.chunk_meta(
            chunk.len() as u32, // doesn't include header size
            scheme,
        ))?;

        self.stream.write_all(chunk)?;
        Ok(())
    }

    fn pad(&mut self) -> fastanvil::Result<()> {
        let current_end = unstable_stream_len(&mut self.stream)? as usize;
        let padded_end = unstable_div_ceil(current_end, SECTOR_SIZE) * SECTOR_SIZE;
        let pad_len = padded_end - current_end;
        self.stream.write_all(&vec![0; pad_len])?;
        Ok(())
    }

    fn set_header(
        &mut self,
        x: usize,
        z: usize,
        offset: u64,
        new_sector_count: usize,
    ) -> fastanvil::Result<()> {
        if new_sector_count > 255 {
            return Err(Error::ChunkTooLarge);
        }

        let mut buf = [0u8; 4];
        buf[0] = ((offset & 0xFF0000) >> 16) as u8;
        buf[1] = ((offset & 0x00FF00) >> 8) as u8;
        buf[2] = (offset & 0x0000FF) as u8;
        buf[3] = new_sector_count as u8;

        // seek to header
        self.stream.seek(SeekFrom::Start(header_pos(x, z)))?;
        self.stream.write_all(&buf)?;
        Ok(())
    }
}

pub struct RegionIter<'a, S> {
    inner: &'a mut Region<S>,
    index: usize,
}

impl<'a, S> RegionIter<'a, S>
where
    S: AsyncReadExt + AsyncSeekExt + Unpin,
{
    async fn async_new(inner: &'a mut Region<S>) -> Self {
        Self { inner, index: 0 }
    }

    async fn async_next_xz(&mut self) -> Option<(usize, usize)> {
        let index = self.index;
        self.index += 1;

        if index == 32 * 32 {
            return None;
        }

        let x = index % 32;
        let z = index / 32;
        Some((x, z))
    }
}

impl<'a, S> Iterator for RegionIter<'a, S>
where
    S: AsyncReadExt + AsyncSeekExt + Unpin,
{
    type Item = fastanvil::Result<ChunkData>;

    fn next(&mut self) -> Option<Self::Item> {
        let handle = tokio::runtime::Handle::current();
        while let Some((x, z)) = handle.block_on(self.async_next_xz()) {
            let c = handle.block_on(self.inner.async_read_chunk(x, z));

            match c {
                Ok(Some(c)) => return Some(Ok(ChunkData { x, z, data: c })),
                Ok(None) => {}
                Err(e) => return Some(Err(e)),
            }
        }

        None
    }
}

impl<'a, S> RegionIter<'a, S>
where
    S: Read + Seek,
{
    fn new(inner: &'a mut Region<S>) -> Self {
        Self { inner, index: 0 }
    }

    fn next_xz(&mut self) -> Option<(usize, usize)> {
        let index = self.index;
        self.index += 1;

        if index == 32 * 32 {
            return None;
        }

        let x = index % 32;
        let z = index / 32;
        Some((x, z))
    }
}
pub struct ChunkData {
    pub x: usize,
    pub z: usize,
    pub data: Vec<u8>,
}

// impl<'a, S> Iterator for RegionIter<'a, S>
// where
//     S: Read + Seek,
// {
//     type Item = fastanvil::Result<ChunkData>;
//
//     fn next(&mut self) -> Option<Self::Item> {
//         while let Some((x, z)) = self.next_xz() {
//             let c = self.inner.read_chunk(x, z);
//
//             match c {
//                 Ok(Some(c)) => return Some(Ok(ChunkData { x, z, data: c })),
//                 Ok(None) => {}
//                 Err(e) => return Some(Err(e)),
//             }
//         }
//
//         None
//     }
// }

#[derive(Debug, TryFromPrimitive)]
#[repr(u8)]
pub enum CompressionScheme {
    Gzip = 1,
    Zlib = 2,
    Uncompressed = 3,
}

pub const fn unstable_div_ceil(lhs: usize, rhs: usize) -> usize {
    let d = lhs / rhs;
    let r = lhs % rhs;
    if r > 0 && rhs > 0 {
        d + 1
    } else {
        d
    }
}

async fn async_unstable_stream_len<S>(seek: &mut S) -> fastanvil::Result<u64>
where
    S: AsyncSeekExt + Unpin,
{
    let old_pos = seek.stream_position().await?;
    let len = seek.seek(SeekFrom::End(0)).await?;
    if old_pos != len {
        seek.seek(SeekFrom::Start(old_pos)).await?;
    }

    Ok(len)
}

fn unstable_stream_len(seek: &mut impl Seek) -> fastanvil::Result<u64> {
    let old_pos = seek.stream_position()?;
    let len = seek.seek(SeekFrom::End(0))?;
    if old_pos != len {
        seek.seek(SeekFrom::Start(old_pos))?;
    }

    Ok(len)
}

fn header_pos(x: usize, z: usize) -> u64 {
    (4 * ((x % 32) + (z % 32) * 32)) as u64
}

#[derive(Debug)]
pub struct ChunkLocation {
    pub offset: u64,
    pub sectors: u64,
}

#[derive(Debug)]
struct ChunkMeta {
    pub compressed_len: u32,
    pub compression_scheme: fastanvil::CompressionScheme,
}

impl ChunkMeta {
    fn new(mut data: &[u8]) -> fastanvil::Result<Self> {
        let len = ReadBytesExt::read_u32::<BigEndian>(&mut data)?;
        let scheme = ReadBytesExt::read_u8(&mut data)?;
        let scheme = fastanvil::CompressionScheme::try_from(scheme)
            .map_err(|_| Error::UnknownCompression(scheme))?;

        Ok(Self {
            compressed_len: len - 1, // this len include the compression byte.
            compression_scheme: scheme,
        })
    }
}
