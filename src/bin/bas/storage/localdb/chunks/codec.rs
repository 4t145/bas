use super::*;
use flate2::{Compression, Decompress, FlushDecompress, Status, write::ZlibEncoder};
use std::io::Write;

const JSON: i64 = 0;
const ZLIB: i64 = 1;
const MIN_COMPRESS_BYTES: usize = 1024;

pub(super) struct Encoded {
    pub codec: i64,
    pub raw_bytes: usize,
    pub payload: Vec<u8>,
}

pub(super) fn encode(events: &[Event]) -> Result<Encoded, Error> {
    let json = serde_json::to_vec(events)?;
    let raw_bytes = json.len();
    if raw_bytes > MAX_BLOCK_BYTES {
        return Err(Error::Corrupt("history block exceeds size limit"));
    }
    if raw_bytes >= MIN_COMPRESS_BYTES {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&json)?;
        let compressed = encoder.finish()?;
        if compressed.len() < raw_bytes {
            return Ok(Encoded {
                codec: ZLIB,
                raw_bytes,
                payload: compressed,
            });
        }
    }
    Ok(Encoded {
        codec: JSON,
        raw_bytes,
        payload: json,
    })
}

pub(super) fn decode(codec: i64, raw_bytes: i64, payload: &[u8]) -> Result<Vec<Event>, Error> {
    let size =
        usize::try_from(raw_bytes).map_err(|_| Error::Corrupt("invalid history block size"))?;
    if size == 0 || (codec != JSON && size > MAX_BLOCK_BYTES) {
        return Err(Error::Corrupt("invalid history block size"));
    }
    match codec {
        JSON if payload.len() == size => Ok(serde_json::from_slice(payload)?),
        ZLIB => {
            let mut decoder = Decompress::new(true);
            let mut json = vec![0; size + 1];
            let status = decoder
                .decompress(payload, &mut json, FlushDecompress::Finish)
                .map_err(|_| Error::Corrupt("invalid compressed history block"))?;
            if status != Status::StreamEnd
                || decoder.total_out() != size as u64
                || decoder.total_in() != payload.len() as u64
            {
                return Err(Error::Corrupt("history block compressed size mismatch"));
            }
            json.truncate(size);
            Ok(serde_json::from_slice(&json)?)
        }
        _ => Err(Error::Corrupt("invalid history block encoding")),
    }
}
