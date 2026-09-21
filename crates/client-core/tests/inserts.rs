//! Late definitions and rebuilds must preserve the insert collision a client predicts.
use client_core::ClientCore;
use protocol::*;
use sim_core::build::{BuildContent, PieceRec, LOC_EDGE_XLO, SHAPE_FRAME, SHAPE_WINDOW};
use sim_core::deploy::{
    DeployContent, DeployRec, ARCH_GARAGE_DOOR, ARCH_WINDOW_BARS, PLACE_FRAME, PLACE_WINDOW,
};

#[test]
fn inserts_survive_late_definitions_and_rebuilds_then_unseal_on_removal() {
    for (arch, placement, shape) in [
        (ARCH_WINDOW_BARS, PLACE_WINDOW, SHAPE_WINDOW),
        (ARCH_GARAGE_DOOR, PLACE_FRAME, SHAPE_FRAME),
    ] {
        let mut c = ClientCore::new(20260731, 7, 0);
        let mut buf = [0; MAX_EVENT_MSG_BYTES];
        let (cx, cz, level, loc) = (341, 341, 2, LOC_EDGE_XLO);
        let sealed = |c: &ClientCore| c.pieces.cols().get(cx, cz).shut_xlo & (1 << level) != 0;
        let mut bc = BuildContent::probe_fixture();
        bc.pieces[3].shape = shape;
        let (n, _) = encode_event_piece_defs(&bc, 0, &mut buf).unwrap();
        c.on_stream(&buf[..n]).unwrap();
        let piece = PieceRec {
            cx,
            cz,
            level,
            loc,
            row: 3,
            ..PieceRec::default()
        };
        let n = encode_event_piece_placed(&piece, &mut buf).unwrap();
        c.on_stream(&buf[..n]).unwrap();
        let insert = DeployRec {
            cx,
            cz,
            level,
            loc,
            row: 2,
            ..DeployRec::default()
        };
        let n = encode_event_deploy_placed(&insert, &mut buf).unwrap();
        c.on_stream(&buf[..n]).unwrap();
        assert!(!sealed(&c), "unknown definitions cannot invent collision");
        let mut dc = DeployContent::probe_fixture();
        dc.defs[2].arch = arch;
        dc.defs[2].placement = placement;
        let (n, _) = encode_event_deploy_defs(&dc, 0, &mut buf).unwrap();
        c.on_stream(&buf[..n]).unwrap();
        assert!(sealed(&c), "late definitions must seal the existing insert");
        let (n, _) = encode_event_piece_defs(&bc, 0, &mut buf).unwrap();
        c.on_stream(&buf[..n]).unwrap();
        assert!(sealed(&c), "a piece-index rebuild must retain inserts");
        if arch == ARCH_WINDOW_BARS {
            assert_eq!(c.predict_door(cx, cz, level, loc), None);
            assert!(sealed(&c), "fixed bars cannot be predicted open");
        } else {
            assert_eq!(c.predict_door(cx, cz, level, loc), Some(true));
            assert!(!sealed(&c));
            let n = encode_event_deploy_refused(sim_core::deploy::REFUSE_D_OWNER as u8, &mut buf)
                .unwrap();
            c.on_stream(&buf[..n]).unwrap();
            assert!(sealed(&c), "a refused garage toggle restores its collision");
        }
        let n = encode_event_removed(false, cx, cz, level, loc, &mut buf).unwrap();
        c.on_stream(&buf[..n]).unwrap();
        assert!(!sealed(&c));
        assert_eq!(
            c.pieces.entries().len(),
            1,
            "removing an insert keeps its frame"
        );
    }
}
