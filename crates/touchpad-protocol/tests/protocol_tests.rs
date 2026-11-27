use touchpad_protocol::{
    ConnectionMethod, HostOs, MouseButton, PairingStatus, PrivilegeMode, ScreenEdge,
    TouchpadPacket as Packet, DEFAULT_PORT,
};

fn assert_round_trip(packet: Packet, wire: &str) {
    assert_eq!(packet.encode(), wire);
    assert_eq!(Packet::decode(wire), Some(packet.clone()));
    let bytes = packet.to_bytes();
    assert_eq!(bytes, wire.as_bytes());
    assert_eq!(Packet::from_bytes(&bytes), Some(packet.clone()));

    let mut buffer = [0; 1024];
    buffer[..bytes.len()].copy_from_slice(&bytes);
    assert_eq!(Packet::from_bytes(&buffer[..bytes.len()]), Some(packet));
}

fn discover_response(name: &str, ip: &str, port: u16, method: ConnectionMethod) -> Packet {
    Packet::DiscoverResponse {
        computer_name: name.into(),
        ip_address: ip.into(),
        port,
        method,
    }
}

#[test]
fn default_port_is_45454() {
    assert_eq!(DEFAULT_PORT, 45454);
}

#[test]
fn v1_round_trips_through_strings_and_bytes() {
    for (dx, dy, wire) in [
        (12.5, -3.75, "MOVE|12.5|-3.75"),
        (0.0, 0.0, "MOVE|0.0|0.0"),
        (0.1234, -0.0005, "MOVE|0.1234|-5.0E-4"),
    ] {
        assert_round_trip(Packet::Move { dx, dy }, wire);
    }

    for (button, pressed, id) in [
        (MouseButton::Left, true, "LEFT"),
        (MouseButton::Right, false, "RIGHT"),
        (MouseButton::Middle, true, "MIDDLE"),
    ] {
        assert_round_trip(Packet::Click { button }, &format!("CLICK|{id}"));
        assert_round_trip(
            Packet::ButtonState { button, pressed },
            &format!("BUTTON_STATE|{id}|{pressed}"),
        );
    }

    for (delta_y, wire) in [(4.25, "SCROLL|4.25"), (-12.5, "SCROLL|-12.5")] {
        assert_round_trip(Packet::Scroll { delta_y }, wire);
    }

    for client_name in ["AndroidTouchpad", "Pixel 8 Pro (João)"] {
        assert_round_trip(
            Packet::DiscoverRequest {
                client_name: client_name.into(),
            },
            &format!("DISCOVER_REQ|{client_name}"),
        );
    }

    for (name, ip, port, method) in [
        (
            "Desktop-João",
            "192.168.1.42",
            DEFAULT_PORT,
            ConnectionMethod::NetworkLan,
        ),
        ("Workstation", "fe80::1", 1, ConnectionMethod::Bluetooth),
        ("Laptop", "10.0.0.5", u16::MAX, ConnectionMethod::UsbAdb),
    ] {
        assert_round_trip(
            discover_response(name, ip, port, method),
            &format!("DISCOVER_RESP|{name}|{ip}|{port}|{}", method.id()),
        );
    }

    for timestamp in [0, i64::MAX, i64::MIN, 1_730_000_000_123] {
        assert_round_trip(Packet::Ping { timestamp }, &format!("PING|{timestamp}"));
    }

    for (timestamp, name) in [
        (1_730_000_000_456, "Servidor Principal"),
        (i64::MAX, "MaxServer"),
        (i64::MIN, "MinServer"),
    ] {
        assert_round_trip(
            Packet::Pong {
                timestamp,
                server_name: name.into(),
            },
            &format!("PONG|{timestamp}|{name}"),
        );
    }

    for (id, name) in [
        ("device-uuid-01", "AndroidTouchpad"),
        ("pixel-8-pro", "Pixel 8 Pro (João)"),
    ] {
        assert_round_trip(
            Packet::PairRequest {
                client_id: id.into(),
                client_name: name.into(),
            },
            &format!("PAIR_REQ|{id}|{name}"),
        );
    }

    for (status, id, name) in [
        (PairingStatus::Pending, "PENDING", "Desktop-João"),
        (PairingStatus::Approved, "APPROVED", "Workstation"),
        (PairingStatus::Rejected, "REJECTED", "Servidor Principal"),
    ] {
        assert_round_trip(
            Packet::PairResponse {
                status,
                server_name: name.into(),
            },
            &format!("PAIR_RESP|{id}|{name}"),
        );
    }
}

#[test]
fn negative_zero_is_normalized() {
    let packet = Packet::Move { dx: -0.0, dy: -0.0 };
    assert_eq!(packet.encode(), "MOVE|0.0|0.0");
    let Some(Packet::Move { dx, dy }) = Packet::decode("MOVE|-0.0|-0.0") else {
        panic!("negative zero should decode");
    };
    assert_eq!((dx.to_bits(), dy.to_bits()), (0, 0));
}

#[test]
fn buffer_boundaries_and_whitespace() {
    let packet = Packet::Pong {
        timestamp: 123456,
        server_name: "Estação-São Paulo".into(),
    };
    let payload = format!(" \t{}\r\n", packet.encode()).into_bytes();
    assert_eq!(Packet::from_bytes(&payload), Some(packet.clone()));
    assert_eq!(Packet::from_bytes(&[]), None);
    assert_eq!(Packet::from_bytes(&payload[..0]), None);
    assert!(payload.get(..payload.len() + 1).is_none());
    assert_eq!(Packet::from_bytes(&[0; 256]), None);
    assert_eq!(Packet::from_bytes(b"PONG|123456|\xff"), None);

    let mut surrounding_bytes = vec![0xff];
    surrounding_bytes.extend_from_slice(&payload);
    surrounding_bytes.push(0xff);
    assert_eq!(
        Packet::from_bytes(&surrounding_bytes[1..1 + payload.len()]),
        Some(packet),
    );
    assert_eq!(Packet::from_bytes(&surrounding_bytes[1..]), None);
}

#[test]
fn connection_method_ids_and_availability() {
    use ConnectionMethod::*;
    for (id, method) in [
        ("NETWORK_LAN", NetworkLan),
        ("network_lan", NetworkLan),
        ("bluetooth", Bluetooth),
        ("  bluetooth \r\n", Bluetooth),
        ("Usb_Adb", UsbAdb),
        ("  usb_adb ", UsbAdb),
        ("", NetworkLan),
        ("UNKNOWN", NetworkLan),
    ] {
        assert_eq!(ConnectionMethod::from_id(id), method);
    }

    for (method, id, available) in [
        (NetworkLan, "NETWORK_LAN", true),
        (Bluetooth, "BLUETOOTH", false),
        (UsbAdb, "USB_ADB", false),
    ] {
        assert_eq!(method.id(), id);
        assert_eq!(method.is_available_now(), available);
    }
}

#[test]
fn sanitizes_names_and_compact_tokens_with_legacy_fallbacks() {
    use ConnectionMethod::NetworkLan;
    for (packet, wire) in [
        (
            Packet::DiscoverRequest {
                client_name: "My|Phone\n\r\t\0Pro".into(),
            },
            "DISCOVER_REQ|My Phone Pro",
        ),
        (
            Packet::DiscoverRequest {
                client_name: "|||\n\r\x07".into(),
            },
            "DISCOVER_REQ|AndroidTouchpad",
        ),
        (
            discover_response(
                "Work|PC\n1\x01",
                " 192.168.|1.10\r\n ",
                DEFAULT_PORT,
                NetworkLan,
            ),
            "DISCOVER_RESP|Work PC 1|192.168.1.10|45454|NETWORK_LAN",
        ),
        (
            discover_response("|||", "10.0.0.1", DEFAULT_PORT, NetworkLan),
            "DISCOVER_RESP|Unknown|10.0.0.1|45454|NETWORK_LAN",
        ),
        (
            Packet::Pong {
                timestamp: 777,
                server_name: "Host|Srv\t\x1fMain".into(),
            },
            "PONG|777|Host Srv Main",
        ),
        (
            Packet::Pong {
                timestamp: 1,
                server_name: "|||".into(),
            },
            "PONG|1|Unknown",
        ),
        (
            Packet::PairRequest {
                client_id: " id|123 \t\n\0abc ".into(),
                client_name: "Tab|S9\r\n\x07Ultra".into(),
            },
            "PAIR_REQ|id123abc|Tab S9 Ultra",
        ),
        (
            Packet::PairRequest {
                client_id: " | \n\t\r ".into(),
                client_name: "|||\0".into(),
            },
            "PAIR_REQ|android-default|AndroidTouchpad",
        ),
        (
            Packet::PairResponse {
                status: PairingStatus::Approved,
                server_name: "Office|PC\n01\x02".into(),
            },
            "PAIR_RESP|APPROVED|Office PC 01",
        ),
        (
            Packet::PairResponse {
                status: PairingStatus::Rejected,
                server_name: "|||\r\n".into(),
            },
            "PAIR_RESP|REJECTED|Unknown",
        ),
        (
            Packet::PairRequest {
                client_id: "a\u{0085}\u{2003}b".into(),
                client_name: "  É\u{009f}lan |  東京  ".into(),
            },
            "PAIR_REQ|ab|Élan 東京",
        ),
    ] {
        assert_eq!(packet.encode(), wire);
        assert_eq!(Packet::decode(wire).unwrap().encode(), wire);
    }

    for raw in ["DISCOVER_REQ|", "DISCOVER_REQ|   "] {
        assert_eq!(
            Packet::decode(raw),
            Some(Packet::DiscoverRequest {
                client_name: "AndroidTouchpad".into()
            })
        );
    }
}

#[test]
fn wire_literals_are_locale_independent() {
    assert_round_trip(
        Packet::Move {
            dx: 12.34,
            dy: -56.78,
        },
        "MOVE|12.34|-56.78",
    );
    assert_round_trip(Packet::Scroll { delta_y: -2.5 }, "SCROLL|-2.5");
    for (raw, canonical) in [
        ("CLICK|middle", "CLICK|MIDDLE"),
        ("BUTTON_STATE|right|TRUE", "BUTTON_STATE|RIGHT|true"),
        (
            "DISCOVER_RESP|PC|192.168.0.2|45454|usb_adb",
            "DISCOVER_RESP|PC|192.168.0.2|45454|USB_ADB",
        ),
        ("PAIR_RESP|pending|PC", "PAIR_RESP|PENDING|PC"),
        ("PAIR_RESP|approved|PC", "PAIR_RESP|APPROVED|PC"),
        ("PAIR_RESP|rejected|PC", "PAIR_RESP|REJECTED|PC"),
    ] {
        assert_eq!(Packet::decode(raw).unwrap().encode(), canonical);
    }
    assert_eq!(Packet::decode("MOVE|12,34|-56,78"), None);
    assert_eq!(Packet::decode("SCROLL|-2,5"), None);
}

#[test]
fn malformed_v1_packets_are_rejected() {
    for raw in [
        "",
        "   ",
        "\r\n\t",
        "UNKNOWN",
        "UNKNOWN|1|2",
        "MOVE",
        "MOVE|1.0",
        "MOVE|1.0|2.0|3.0",
        "MOVE|abc|1.0",
        "MOVE|1.0|xyz",
        "MOVE||1.0",
        "MOVE|1.0\0|2.0",
        "MOVE|NaN|1.0",
        "MOVE|1.0|Infinity",
        "MOVE|-Infinity|0.0",
        "SCROLL",
        "SCROLL|1.0|2.0",
        "SCROLL|NaN",
        "SCROLL|Infinity",
        "SCROLL|not_a_float",
        "CLICK",
        "CLICK|",
        "CLICK|SIDE",
        "CLICK|LEFT|EXTRA",
        "BUTTON_STATE",
        "BUTTON_STATE|LEFT",
        "BUTTON_STATE|LEFT|yes",
        "BUTTON_STATE|LEFT|1",
        "BUTTON_STATE|LEFT|",
        "BUTTON_STATE|UNKNOWN|true",
        "BUTTON_STATE|LEFT|true|extra",
        "DISCOVER_REQ",
        "DISCOVER_REQ|Phone|Extra",
        "DISCOVER_RESP",
        "DISCOVER_RESP|PC|192.168.1.1|45454",
        "DISCOVER_RESP||192.168.1.1|45454|NETWORK_LAN",
        "DISCOVER_RESP|PC||45454|NETWORK_LAN",
        "DISCOVER_RESP|PC|192.168.1.1|0|NETWORK_LAN",
        "DISCOVER_RESP|PC|192.168.1.1|-1|NETWORK_LAN",
        "DISCOVER_RESP|PC|192.168.1.1|65536|NETWORK_LAN",
        "DISCOVER_RESP|PC|192.168.1.1|abc|NETWORK_LAN",
        "DISCOVER_RESP|PC|192.168.1.1|45454|",
        "DISCOVER_RESP|PC|192.168.1.1|45454|NETWORK_LAN|EXTRA",
        "PING",
        "PING|",
        "PING|abc",
        "PING|9999999999999999999999",
        "PING|123|456",
        "PONG",
        "PONG|123",
        "PONG|abc|Server",
        "PONG|9999999999999999999999|Server",
        "PONG|123|",
        "PONG|123|   ",
        "PONG|123|Server|Extra",
        "PAIR_REQ",
        "PAIR_REQ|client-1",
        "PAIR_REQ||Phone",
        "PAIR_REQ|   |Phone",
        "PAIR_REQ|client-1|",
        "PAIR_REQ|client-1|   ",
        "PAIR_REQ|client-1|Phone|Extra",
        "PAIR_RESP",
        "PAIR_RESP|PENDING",
        "PAIR_RESP|PENDING|",
        "PAIR_RESP|PENDING|   ",
        "PAIR_RESP|UNKNOWN|Server",
        "PAIR_RESP||Server",
        "PAIR_RESP|PENDING\0|Server",
        "PAIR_RESP|APPROVED|Server|Extra",
        "DISCOVER_RESP|PC|192.168.1.1|70000|NETWORK_LAN",
    ] {
        assert_eq!(Packet::decode(raw), None, "{raw:?}");
    }

    for packet in [
        Packet::Move {
            dx: f32::NAN,
            dy: 1.0,
        },
        Packet::Move {
            dx: 1.0,
            dy: f32::INFINITY,
        },
        Packet::Scroll {
            delta_y: f32::NEG_INFINITY,
        },
        discover_response("PC", "192.168.1.1", 0, ConnectionMethod::NetworkLan),
    ] {
        assert_eq!(Packet::decode(&packet.encode()), None);
    }
}

#[test]
fn unknown_discovery_method_falls_back_to_network_lan() {
    assert_eq!(
        Packet::decode("DISCOVER_RESP|MyPC|192.168.1.20|45454|FUTURE_LINK"),
        Some(discover_response(
            "MyPC",
            "192.168.1.20",
            DEFAULT_PORT,
            ConnectionMethod::NetworkLan
        )),
    );
}

#[test]
fn v2_round_trips_through_strings_and_bytes() {
    assert_round_trip(
        Packet::EdgeConfig {
            left: true,
            right: false,
            top: true,
            bottom: false,
            overshoot_px: 0,
        },
        "EDGE_CONFIG|true|false|true|false|0",
    );
    assert_round_trip(
        Packet::EdgeConfig {
            left: false,
            right: true,
            top: false,
            bottom: true,
            overshoot_px: u16::MAX,
        },
        "EDGE_CONFIG|false|true|false|true|65535",
    );

    use ScreenEdge::*;
    for (edge, id, opposite) in [
        (Left, "LEFT", Right),
        (Right, "RIGHT", Left),
        (Top, "TOP", Bottom),
        (Bottom, "BOTTOM", Top),
    ] {
        assert_eq!(edge.opposite(), opposite);
        assert_round_trip(
            Packet::EdgeHit { edge, ratio: 0.25 },
            &format!("EDGE_HIT|{id}|0.25"),
        );
        assert_round_trip(
            Packet::CursorEnter { edge, ratio: 0.75 },
            &format!("CURSOR_ENTER|{id}|0.75"),
        );
        assert_round_trip(Packet::CursorLeave { edge }, &format!("CURSOR_LEAVE|{id}"));
    }

    use PrivilegeMode::*;
    for (os, width, height, privilege, wire) in [
        (
            HostOs::Windows,
            1920,
            1080,
            User,
            "HOST_INFO|WINDOWS|1920|1080|USER",
        ),
        (HostOs::Windows, 1, 1, Admin, "HOST_INFO|WINDOWS|1|1|ADMIN"),
        (
            HostOs::Windows,
            u32::MAX,
            u32::MAX,
            System,
            "HOST_INFO|WINDOWS|4294967295|4294967295|SYSTEM",
        ),
        (
            HostOs::Linux,
            2560,
            1440,
            Uinput,
            "HOST_INFO|LINUX|2560|1440|UINPUT",
        ),
        (
            HostOs::Linux,
            3840,
            2160,
            Xtest,
            "HOST_INFO|LINUX|3840|2160|XTEST",
        ),
    ] {
        assert_round_trip(
            Packet::HostInfo {
                os,
                width,
                height,
                privilege,
            },
            wire,
        );
    }
}

#[test]
fn v2_ratios_are_clamped_on_encode_and_decode() {
    let edge = ScreenEdge::Left;
    for (raw_ratio, value, canonical) in
        [(-10.0, 0.0, "0.0"), (-0.0, 0.0, "0.0"), (4.0, 1.0, "1.0")]
    {
        assert_eq!(
            Packet::EdgeHit {
                edge,
                ratio: raw_ratio
            }
            .encode(),
            format!("EDGE_HIT|LEFT|{canonical}"),
        );
        assert_eq!(
            Packet::CursorEnter {
                edge,
                ratio: raw_ratio
            }
            .encode(),
            format!("CURSOR_ENTER|LEFT|{canonical}"),
        );
        assert_eq!(
            Packet::decode(&format!("EDGE_HIT|LEFT|{raw_ratio}")),
            Some(Packet::EdgeHit { edge, ratio: value }),
        );
        assert_eq!(
            Packet::decode(&format!("CURSOR_ENTER|LEFT|{raw_ratio}")),
            Some(Packet::CursorEnter { edge, ratio: value }),
        );
    }

    for ratio in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for packet in [
            Packet::EdgeHit { edge, ratio },
            Packet::CursorEnter { edge, ratio },
        ] {
            assert_eq!(Packet::decode(&packet.encode()), None);
        }
    }
}

#[test]
fn malformed_v2_packets_are_rejected() {
    for raw in [
        "EDGE_CONFIG|true|false|true|false",
        "EDGE_CONFIG|true|false|true|false|1|extra",
        "EDGE_CONFIG|true|false|true|no|1",
        "EDGE_CONFIG|true|false|true|false|-1",
        "EDGE_CONFIG|true|false|true|false|65536",
        "EDGE_HIT|LEFT",
        "EDGE_HIT|NORTH|0.5",
        "EDGE_HIT|LEFT|0.5|extra",
        "EDGE_HIT|LEFT|0,5",
        "EDGE_HIT|LEFT|1e100",
        "EDGE_HIT|LEFT|NaN",
        "EDGE_HIT|LEFT|Infinity",
        "CURSOR_ENTER|RIGHT|Inf",
        "CURSOR_ENTER|RIGHT|-Infinity",
        "CURSOR_ENTER|RIGHT|\0",
        "CURSOR_ENTER|RIGHT|0,5",
        "CURSOR_ENTER||0.5",
        "CURSOR_LEAVE|",
        "CURSOR_LEAVE|WEST",
        "CURSOR_LEAVE|RIGHT|0.5",
        "HOST_INFO|MACOS|1024|768|USER",
        "HOST_INFO|LINUX|0|768|USER",
        "HOST_INFO|LINUX|1024|0|USER",
        "HOST_INFO|LINUX|-1|768|USER",
        "HOST_INFO|LINUX|1024|4294967296|USER",
        "HOST_INFO|LINUX|1024|768|ROOT",
        "HOST_INFO|LINUX|1024|768",
        "HOST_INFO|LINUX|1024|768|USER|extra",
    ] {
        assert_eq!(Packet::decode(raw), None, "{raw:?}");
    }
}
