use protocol::{decode_event, encode_event_chunk, EventMsg, WireError};

#[test]
fn chunk_lifecycle_goldens_and_strict_bounds() {
    let fixtures: [&[u8]; 4] = [
        include_bytes!("golden/event_chunk_0.bin"),
        include_bytes!("golden/event_chunk_1.bin"),
        include_bytes!("golden/event_chunk_2.bin"),
        include_bytes!("golden/event_chunk_3.bin"),
    ];
    let mut buf = [0u8; 32];
    for (state, golden) in fixtures.iter().enumerate() {
        let (chunk, version) = if state == 3 {
            (0, 0)
        } else {
            (321, 0x12345678)
        };
        let n = encode_event_chunk(chunk, version, state as u8, &mut buf).unwrap();
        assert_eq!(&buf[..n], *golden);
        assert_eq!(
            decode_event(golden).unwrap(),
            EventMsg::Chunk {
                chunk,
                version,
                state: state as u8,
            }
        );
        for truncated in 0..n {
            assert!(decode_event(&buf[..truncated]).is_err());
        }
        assert!(decode_event(&buf[..n + 1]).is_err());
        buf[n - 1] |= 0x80;
        assert!(decode_event(&buf[..n]).is_err(), "nonzero spare bits");
    }
    assert_eq!(
        encode_event_chunk(protocol::chunk::COUNT as u16, 0, 0, &mut buf),
        Err(WireError::Malformed)
    );
    assert_eq!(
        encode_event_chunk(1, 0, 3, &mut buf),
        Err(WireError::Malformed)
    );
    assert_eq!(
        encode_event_chunk(0, 1, 3, &mut buf),
        Err(WireError::Malformed)
    );
    assert_eq!(
        encode_event_chunk(0, 0, 4, &mut buf),
        Err(WireError::Malformed)
    );
}
