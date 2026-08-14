use std::fs::File;
use std::io::{BufReader, Read, Seek};

use byteorder::{self, ReadBytesExt};
use flate2::read::ZlibDecoder;

use crate::proto::fileformat::{Blob, BlobHeader};
use crate::proto::osmformat::{HeaderBlock, PrimitiveBlock};

/// Upper bound for a serialized BlobHeader (matches osmosis's FileBlockHead limit of 64 KiB).
const MAX_HEADER_SIZE: u64 = 65536;
/// Upper bound for a compressed blob body (matches osmosis's FileBlockHead limit of 32 MiB).
/// Bounds memory allocations from untrusted datasize fields.
const MAX_BLOB_SIZE: usize = 33554432;

pub enum DecodedBlob {
    OsmHeader(HeaderBlock),
    OsmData(PrimitiveBlock),
}

#[derive(Debug)]
pub struct RawBlob {
    header: BlobHeader,
    raw_blob: Vec<u8>,
}

impl RawBlob {
    /// Decodes the blob into an OSM header or data block. Returns `Ok(None)` for blob types
    /// this crate does not know about (they are skipped, like osmosis does) and `Err` for
    /// known types whose contents fail to decode.
    pub fn decode(&self) -> anyhow::Result<Option<DecodedBlob>> {
        let decoded = match self.header.get_field_type() {
            "OSMHeader" => Some(DecodedBlob::OsmHeader(self.decode_blob()?)),
            "OSMData" => Some(DecodedBlob::OsmData(self.decode_blob()?)),
            _ => None,
        };
        Ok(decoded)
    }

    fn decode_blob<M: protobuf::Message>(&self) -> anyhow::Result<M> {
        let blob: Blob = protobuf::Message::parse_from_bytes(self.raw_blob.as_slice())?;
        let decoded: M = if blob.has_raw() {
            protobuf::Message::parse_from_bytes(blob.get_raw())?
        } else if blob.has_zlib_data() {
            let mut decoder = ZlibDecoder::new(blob.get_zlib_data());
            protobuf::Message::parse_from_reader(&mut decoder)?
        } else if blob.has_lz4_data() {
            // The lz4 block format is not self-describing; the uncompressed size comes from
            // the blob's raw_size field.
            let decompressed =
                lz4_flex::block::decompress(blob.get_lz4_data(), blob.get_raw_size() as usize)
                    .map_err(|e| anyhow!("failed to decompress lz4 blob: {}", e))?;
            protobuf::Message::parse_from_bytes(&decompressed)?
        } else if blob.has_zstd_data() {
            let decompressed = zstd::stream::decode_all(blob.get_zstd_data())
                .map_err(|e| anyhow!("failed to decompress zstd blob: {}", e))?;
            protobuf::Message::parse_from_bytes(&decompressed)?
        } else {
            // lzma_data is not supported (requires a C toolchain via xz2); deprecated
            // bzip2 is intentionally unsupported.
            bail!("Unsupported blob data type")
        };
        Ok(decoded)
    }
}

pub struct BlobReader<R: Read + Send> {
    reader: R,
    pub offset: u64,
    pub eof: bool,
}

impl<R: Read + Send> BlobReader<R> {
    pub fn new(reader: R) -> BlobReader<R> {
        Self {
            reader,
            offset: 0,
            eof: false,
        }
    }

    pub(crate) fn next_blob(&mut self) -> anyhow::Result<Option<RawBlob>> {
        let header_size = match self.reader.read_u32::<byteorder::BigEndian>() {
            Ok(n) => {
                self.offset += 4;
                n as u64
            }
            Err(ref err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                self.eof = true;
                return Ok(None);
            }
            Err(_) => {
                bail!("Unable to get next blob from PBF stream.");
            }
        };

        let header = self.read_blob_header(header_size)?;
        let raw_blob = self.read_blob(&header)?;
        Ok(Some(RawBlob { header, raw_blob }))
    }

    fn read_blob_header(&mut self, header_size: u64) -> anyhow::Result<BlobHeader> {
        if header_size > MAX_HEADER_SIZE {
            bail!(
                "Unexpectedly long blob header ({} bytes, limit {}). Possibly corrupt file.",
                header_size,
                MAX_HEADER_SIZE
            );
        }
        let header: BlobHeader =
            protobuf::Message::parse_from_reader(&mut self.reader.by_ref().take(header_size))?;
        self.offset += header_size;
        Ok(header)
    }

    fn read_blob(&mut self, header: &BlobHeader) -> anyhow::Result<Vec<u8>> {
        let data_size = header.get_datasize() as usize;
        if data_size > MAX_BLOB_SIZE {
            bail!(
                "Unexpectedly long blob body ({} bytes, limit {}). Possibly corrupt file.",
                data_size,
                MAX_BLOB_SIZE
            );
        }
        let mut bytes = vec![0u8; data_size];
        self.reader.by_ref().read_exact(&mut bytes)?;
        self.offset += data_size as u64;
        Ok(bytes)
    }
}

impl BlobReader<BufReader<File>> {
    pub fn seek(&mut self, offset: u64) -> anyhow::Result<()> {
        self.reader.seek(std::io::SeekFrom::Start(offset))?;
        self.offset = offset;
        Ok(())
    }

    pub fn rewind(&mut self) -> anyhow::Result<()> {
        self.reader.rewind()?;
        self.offset = 0;
        Ok(())
    }
}

impl<R: Read + Send> Iterator for BlobReader<R> {
    /// A convenience iterator over blobs. I/O or decode-framing errors are yielded as `Err`
    /// items instead of panicking, so `par_bridge`-style consumers can propagate them.
    /// Prefer `next_blob()` when you need the error without ending iteration on it.
    type Item = anyhow::Result<RawBlob>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.eof {
            None
        } else {
            match self.next_blob() {
                Ok(Some(raw)) => Some(Ok(raw)),
                Ok(None) => None,
                Err(err) => {
                    self.eof = true;
                    Some(Err(err))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::osmformat::HeaderBlock;
    use protobuf::Message;

    fn wrap(blob: Blob, blob_type: &str) -> RawBlob {
        let mut header = BlobHeader::new();
        header.set_datasize(blob.write_to_bytes().unwrap().len() as i32);
        header.set_field_type(blob_type.to_owned());
        RawBlob {
            header,
            raw_blob: blob.write_to_bytes().unwrap(),
        }
    }

    #[test]
    fn decode_lz4_blob() {
        let raw = HeaderBlock::new().write_to_bytes().unwrap();
        let compressed = lz4_flex::block::compress(&raw);
        let mut blob = Blob::new();
        blob.set_lz4_data(compressed);
        blob.set_raw_size(raw.len() as i32);
        let decoded = wrap(blob, "OSMHeader").decode().unwrap();
        assert!(matches!(decoded, Some(DecodedBlob::OsmHeader(_))));
    }

    #[test]
    fn decode_zstd_blob() {
        let raw = HeaderBlock::new().write_to_bytes().unwrap();
        let compressed = zstd::stream::encode_all(&raw[..], 3).unwrap();
        let mut blob = Blob::new();
        blob.set_zstd_data(compressed);
        blob.set_raw_size(raw.len() as i32);
        let decoded = wrap(blob, "OSMHeader").decode().unwrap();
        assert!(matches!(decoded, Some(DecodedBlob::OsmHeader(_))));
    }

    #[test]
    fn decode_unknown_blob_type_returns_none() {
        let raw = HeaderBlock::new().write_to_bytes().unwrap();
        let mut blob = Blob::new();
        blob.set_raw(raw);
        let decoded = wrap(blob, "SomethingNew").decode().unwrap();
        assert!(decoded.is_none());
    }

    #[test]
    fn blob_header_size_limit_is_enforced() {
        let mut reader = BlobReader::new(std::io::empty());
        let err = reader.read_blob_header(MAX_HEADER_SIZE + 1).unwrap_err();
        assert!(err.to_string().contains("header"));
    }

    #[test]
    fn blob_body_size_limit_is_enforced() {
        let mut header = BlobHeader::new();
        header.set_datasize((MAX_BLOB_SIZE + 1) as i32);
        let mut reader = BlobReader::new(std::io::empty());
        let err = reader.read_blob(&header).unwrap_err();
        assert!(err.to_string().contains("blob body"));
    }
}
