// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Wire framing — 4-byte big-endian length prefix + CBOR payload.
//!
//! Per [`super`] module docs. The length-prefix approach gives
//! deterministic recovery: a malformed CBOR payload doesn't desync the
//! stream because the next frame's start is always at a known offset.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::{Read, Write};
use thiserror::Error;

/// Default ceiling for one frame. Matches the largest reasonable
/// `RuntimePackageMirror` archive (a fully-mirrored Julia core
/// ontology comes in well under this) while still being small enough
/// that an attacker-supplied length-prefix can't trigger gigabyte
/// allocations.
pub const MAX_FRAME_SIZE_DEFAULT: usize = 64 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum FrameError {
    /// Underlying I/O failed (broken pipe, premature EOF, etc.).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// CBOR encode failure — only happens for runtime-specific
    /// edge cases (e.g. integer overflow) since serde derives are
    /// infallible on the wire types defined in [`super::protocol`].
    #[error("CBOR encode failed: {0}")]
    Encode(String),

    /// CBOR decode failure — malformed bytes or schema mismatch.
    #[error("CBOR decode failed: {0}")]
    Decode(String),

    /// The advertised frame length exceeds the configured ceiling.
    /// The reader closes the connection rather than allocating.
    #[error("frame too large: {got} bytes (max {max})")]
    FrameTooLarge { got: usize, max: usize },

    /// The reader hit EOF before a complete frame was available.
    /// Distinct from [`FrameError::Io`] so callers can decide whether
    /// EOF is benign (peer closed cleanly between frames) or fatal
    /// (peer closed mid-frame).
    #[error("unexpected EOF after {0} bytes of {1}-byte frame")]
    UnexpectedEof(usize, usize),
}

/// Encode a CBOR-serializable value as a length-prefixed frame and
/// write it to `out`. Returns the number of bytes written (header +
/// body).
pub fn encode_frame<T: Serialize, W: Write>(value: &T, out: &mut W) -> Result<usize, FrameError> {
    let mut body = Vec::new();
    ciborium::into_writer(value, &mut body).map_err(|e| FrameError::Encode(e.to_string()))?;
    let len: u32 = body
        .len()
        .try_into()
        .map_err(|_| FrameError::Encode(format!("frame body too large: {}", body.len())))?;
    out.write_all(&len.to_be_bytes())?;
    out.write_all(&body)?;
    Ok(4 + body.len())
}

/// Decode the next length-prefixed frame from `inp`.
///
/// Returns `Ok(None)` if the reader is at clean EOF *between* frames
/// (peer closed gracefully). Returns `Err` for any other failure
/// including mid-frame EOF.
pub fn decode_frame<T: DeserializeOwned, R: Read>(
    inp: &mut R,
    max_frame_size: usize,
) -> Result<Option<T>, FrameError> {
    let mut header = [0u8; 4];
    let mut header_filled = 0;
    while header_filled < 4 {
        match inp.read(&mut header[header_filled..])? {
            // Zero-byte read at offset 0 = clean EOF between frames.
            0 if header_filled == 0 => return Ok(None),
            // Zero-byte read mid-header = peer disconnected after
            // sending a partial length prefix.
            0 => return Err(FrameError::UnexpectedEof(header_filled, 4)),
            n => header_filled += n,
        }
    }
    let len = u32::from_be_bytes(header) as usize;
    if len > max_frame_size {
        return Err(FrameError::FrameTooLarge {
            got: len,
            max: max_frame_size,
        });
    }
    let mut body = vec![0u8; len];
    let mut filled = 0;
    while filled < len {
        match inp.read(&mut body[filled..])? {
            0 => return Err(FrameError::UnexpectedEof(filled, len)),
            n => filled += n,
        }
    }
    let value = ciborium::from_reader(&body[..]).map_err(|e| FrameError::Decode(e.to_string()))?;
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::protocol::{HealthInfo, NumericalMetadata, Request, Response};
    use std::io::Cursor;

    #[test]
    fn round_trip_request_through_frame() {
        let req = Request::Instantiate {
            env_iri: "urn:eigenius:test:env".to_string(),
            image_digest: None,
        };
        let mut buf = Vec::new();
        let written = encode_frame(&req, &mut buf).expect("encode");
        assert_eq!(written, buf.len());
        let mut cursor = Cursor::new(&buf);
        let decoded: Request = decode_frame(&mut cursor, MAX_FRAME_SIZE_DEFAULT)
            .expect("decode")
            .expect("frame present");
        assert_eq!(decoded, req);
    }

    #[test]
    fn round_trip_response_through_frame() {
        let resp = Response::Health(HealthInfo {
            manifest_hash_in_image: Some("h".to_string()),
            env_digest_in_image: None,
            numerical_metadata: NumericalMetadata::default(),
        });
        let mut buf = Vec::new();
        encode_frame(&resp, &mut buf).expect("encode");
        let mut cursor = Cursor::new(&buf);
        let decoded: Response = decode_frame(&mut cursor, MAX_FRAME_SIZE_DEFAULT)
            .expect("decode")
            .expect("frame present");
        assert_eq!(decoded, resp);
    }

    #[test]
    fn header_carries_big_endian_length() {
        let req = Request::Health;
        let mut buf = Vec::new();
        encode_frame(&req, &mut buf).expect("encode");
        let header = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        assert_eq!(header as usize, buf.len() - 4);
    }

    #[test]
    fn multiple_frames_decode_in_order() {
        let mut buf = Vec::new();
        encode_frame(&Request::Health, &mut buf).expect("encode 1");
        encode_frame(&Request::Evict, &mut buf).expect("encode 2");
        let mut cursor = Cursor::new(&buf);
        let first: Request = decode_frame(&mut cursor, MAX_FRAME_SIZE_DEFAULT)
            .expect("decode 1")
            .expect("frame 1");
        let second: Request = decode_frame(&mut cursor, MAX_FRAME_SIZE_DEFAULT)
            .expect("decode 2")
            .expect("frame 2");
        assert_eq!(first, Request::Health);
        assert_eq!(second, Request::Evict);
    }

    #[test]
    fn clean_eof_between_frames_is_ok_none() {
        let mut empty = Cursor::new(Vec::<u8>::new());
        let result: Option<Request> =
            decode_frame(&mut empty, MAX_FRAME_SIZE_DEFAULT).expect("decode");
        assert!(result.is_none());
    }

    #[test]
    fn partial_header_then_eof_is_unexpected_eof() {
        // Two header bytes, then EOF. Should report mid-frame EOF
        // rather than clean EOF (which is reserved for zero header
        // bytes consumed).
        let mut partial = Cursor::new(vec![0u8, 0u8]);
        let err = decode_frame::<Request, _>(&mut partial, MAX_FRAME_SIZE_DEFAULT)
            .expect_err("decode should fail");
        assert!(
            matches!(err, FrameError::UnexpectedEof(2, 4)),
            "expected UnexpectedEof(2, 4), got {err:?}"
        );
    }

    #[test]
    fn truncated_body_is_unexpected_eof() {
        // Header says 100-byte body, but only 5 bytes follow.
        let mut buf = Vec::new();
        buf.extend_from_slice(&100u32.to_be_bytes());
        buf.extend_from_slice(&[0u8; 5]);
        let mut cursor = Cursor::new(buf);
        let err = decode_frame::<Request, _>(&mut cursor, MAX_FRAME_SIZE_DEFAULT)
            .expect_err("decode should fail");
        assert!(matches!(err, FrameError::UnexpectedEof(5, 100)));
    }

    #[test]
    fn oversized_frame_is_rejected() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(MAX_FRAME_SIZE_DEFAULT as u32 + 1).to_be_bytes());
        let mut cursor = Cursor::new(buf);
        let err = decode_frame::<Request, _>(&mut cursor, MAX_FRAME_SIZE_DEFAULT)
            .expect_err("oversized frame should fail");
        assert!(
            matches!(err, FrameError::FrameTooLarge { .. }),
            "expected FrameTooLarge, got {err:?}"
        );
    }

    #[test]
    fn malformed_cbor_body_is_decode_error() {
        // Header says 1 byte, body is `0xff` which is the CBOR
        // `break` stop-code outside any indefinite-length context.
        let mut buf = Vec::new();
        buf.extend_from_slice(&1u32.to_be_bytes());
        buf.push(0xff);
        let mut cursor = Cursor::new(buf);
        let err = decode_frame::<Request, _>(&mut cursor, MAX_FRAME_SIZE_DEFAULT)
            .expect_err("malformed CBOR should fail");
        assert!(matches!(err, FrameError::Decode(_)));
    }

    #[test]
    fn custom_max_frame_size_enforced() {
        // Encode a 100-byte frame, then reject with a 32-byte cap.
        let req = Request::RegisterMirror {
            mirror_iri: "x".to_string(),
            library_content: serde_bytes::ByteBuf::from(vec![0u8; 100]),
        };
        let mut buf = Vec::new();
        encode_frame(&req, &mut buf).expect("encode");
        let mut cursor = Cursor::new(buf);
        let err =
            decode_frame::<Request, _>(&mut cursor, 32).expect_err("should fail under tight cap");
        match err {
            FrameError::FrameTooLarge { max, .. } => assert_eq!(max, 32),
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
