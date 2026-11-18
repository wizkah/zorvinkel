use zorvinkel_protocol::status::*;

#[test]
fn request_and_response_variants_roundtrip() {
    for operation in [
        Operation::Register,
        Operation::Authenticate,
        Operation::Status,
        Operation::Capabilities,
        Operation::Unknown(99),
    ] {
        let request = Request { id: 42, operation };
        assert_eq!(
            decode_request(&encode_request(request).unwrap()).unwrap(),
            request
        );
    }
    for body in [
        ResponseBody::Registered,
        ResponseBody::Authenticated,
        ResponseBody::Status {
            mode: 0,
            ssid: String::new(),
        },
        ResponseBody::Status {
            mode: 1,
            ssid: "WLAN".into(),
        },
        ResponseBody::Status {
            mode: 2,
            ssid: "Device AP".into(),
        },
        ResponseBody::Capabilities(CAPABILITIES),
        ResponseBody::Error(RemoteError::Unauthorized),
        ResponseBody::Error(RemoteError::RegistrationClosed),
        ResponseBody::Error(RemoteError::Unsupported),
        ResponseBody::Error(RemoteError::Unavailable),
    ] {
        let response = Response { id: 42, body };
        assert_eq!(
            decode_response(&encode_response(&response).unwrap()).unwrap(),
            response
        );
    }
}

#[test]
fn versions_lengths_zero_ids_and_trailing_data_are_rejected() {
    let valid = encode_request(Request {
        id: 1,
        operation: Operation::Register,
    })
    .unwrap();
    for length in 0..valid.len() {
        assert!(decode_request(&valid[..length]).is_err());
    }
    for index in [0, 2, 3, 4, 14] {
        let mut changed = valid.clone();
        changed[index] = if index == 4 { 0 } else { 7 };
        assert!(decode_request(&changed).is_err());
    }
    let mut combined = valid.clone();
    combined.extend(&valid);
    assert!(decode_request(&combined).is_err());
    assert!(decode_request(&vec![0; MAX_PLAINTEXT + 1]).is_err());
}

#[test]
fn invalid_statuses_and_response_lengths_are_rejected() {
    for (mode, ssid) in [
        (0, "not empty".into()),
        (1, "".into()),
        (2, "x".repeat(33)),
        (3, "network".into()),
        (1, "line\nbreak".into()),
    ] {
        assert!(
            encode_response(&Response {
                id: 1,
                body: ResponseBody::Status { mode, ssid }
            })
            .is_err()
        );
    }
    let valid = encode_response(&Response {
        id: 1,
        body: ResponseBody::Status {
            mode: 1,
            ssid: "é".into(),
        },
    })
    .unwrap();
    let mut invalid_utf8 = valid.clone();
    *invalid_utf8.last_mut().unwrap() = 0xff;
    assert!(decode_response(&invalid_utf8).is_err());
    for length in 0..valid.len() {
        assert!(decode_response(&valid[..length]).is_err());
    }
}

#[test]
fn bounded_records_reject_truncation_concatenation_and_oversize() {
    let record = wrap_record(&vec![7; MAX_RECORD - 4]).unwrap();
    assert_eq!(record.len(), MAX_RECORD);
    assert_eq!(record_payload(&record).unwrap().len(), MAX_RECORD - 4);
    for length in [0, 1, 3, 4, MAX_RECORD - 1] {
        assert!(record_payload(&record[..length]).is_err());
    }
    let mut combined = wrap_record(b"message").unwrap();
    combined.extend(wrap_record(b"other").unwrap());
    assert!(record_payload(&combined).is_err());
    assert!(wrap_record(&[]).is_err());
    assert!(wrap_record(&vec![0; MAX_RECORD - 3]).is_err());
}

#[test]
fn wifi_credentials_and_tasks_are_bounded_and_typed() {
    use zorvinkel_protocol::wifi::*;
    let payload = connect_payload(17, "Café", "test-password").unwrap();
    let request = Request {
        id: 2,
        operation: Operation::WifiConnect,
    };
    let encoded = encode_request_payload(request, &payload).unwrap();
    assert_eq!(decode_request(&encoded).unwrap(), request);
    let input = read_connect(request_payload(&encoded).unwrap()).unwrap();
    assert_eq!(
        (input.id, input.ssid, input.password),
        (17, "Café", "test-password")
    );
    for length in 0..payload.len() {
        assert!(read_connect(&payload[..length]).is_err());
    }
    assert!(connect_payload(0, "AP", "test-password").is_err());
    assert!(connect_payload(1, "AP\nSAVE_CONFIG", "test-password").is_err());
    assert!(connect_payload(1, "AP", "bad").is_err());
    for state in 0..=5 {
        let task = WifiTask {
            id: 17,
            state,
            networks: if state == 1 {
                vec!["Café".into()]
            } else {
                vec![]
            },
        };
        let response = Response {
            id: 2,
            body: ResponseBody::WifiTask(task.clone()),
        };
        assert_eq!(
            decode_response(&encode_response(&response).unwrap()).unwrap(),
            response
        );
        let bytes = encode_task(&task).unwrap();
        for length in 0..bytes.len() {
            assert!(decode_task(&bytes[..length]).is_err());
        }
    }
    assert!(
        encode_task(&WifiTask {
            id: 1,
            state: 1,
            networks: vec!["AP".into(); 65]
        })
        .is_err()
    );
    assert!(
        encode_task(&WifiTask {
            id: 1,
            state: 2,
            networks: vec!["AP".into()]
        })
        .is_err()
    );
    for operation in [Operation::WifiScan, Operation::WifiPoll] {
        let request = Request { id: 1, operation };
        assert!(encode_request(request).is_err());
        let bytes = encode_request_payload(request, &id_payload(7).unwrap()).unwrap();
        assert_eq!(decode_request(&bytes).unwrap(), request);
    }
}

#[test]
fn wifi_persistence_messages_are_typed_and_bounded() {
    use zorvinkel_protocol::wifi::*;
    for change in [Some(false), Some(true), None] {
        let payload = persistence_payload(change);
        assert_eq!(read_persistence(&payload).unwrap(), change);
        let request = Request {
            id: 3,
            operation: Operation::WifiPersistence,
        };
        let encoded = encode_request_payload(request, &payload).unwrap();
        assert_eq!(decode_request(&encoded).unwrap(), request);
    }
    for payload in [&[][..], &[3], &[0, 0]] {
        assert!(read_persistence(payload).is_err());
    }
    for enabled in [false, true] {
        let response = Response {
            id: 4,
            body: ResponseBody::WifiPersistence(enabled),
        };
        assert_eq!(
            decode_response(&encode_response(&response).unwrap()).unwrap(),
            response
        );
    }
}

#[test]
fn power_messages_are_typed_and_bounded() {
    use zorvinkel_protocol::power::*;
    for command in [PowerCommand::Shutdown, PowerCommand::Restart] {
        let payload = power_payload(command);
        assert_eq!(read_power(&payload).unwrap(), command);
        let request = Request {
            id: 5,
            operation: Operation::Power,
        };
        let encoded = encode_request_payload(request, &payload).unwrap();
        assert_eq!(decode_request(&encoded).unwrap(), request);
        let response = Response {
            id: 5,
            body: ResponseBody::PowerAccepted(command),
        };
        assert_eq!(
            decode_response(&encode_response(&response).unwrap()).unwrap(),
            response
        );
    }
    for payload in [&[][..], &[0], &[3], &[1, 1]] {
        assert!(read_power(payload).is_err());
    }
    let request = Request {
        id: 5,
        operation: Operation::Power,
    };
    assert!(encode_request(request).is_err());
    assert!(
        encode_request_payload(
            Request {
                id: 5,
                operation: Operation::Unknown(9)
            },
            &[1]
        )
        .is_err()
    );
}

#[test]
fn access_point_messages_are_typed_bounded_and_redacted() {
    use zorvinkel_protocol::ap::*;
    // Capability bits must stay distinct from every other feature.
    let others = [
        CAPABILITIES,
        zorvinkel_protocol::wifi::CAP_WIFI_TEMPORARY,
        zorvinkel_protocol::wifi::CAP_WIFI_PERSISTENCE,
        zorvinkel_protocol::power::CAP_POWER,
    ];
    assert!(others.iter().all(|bits| bits & CAP_ACCESS_POINT == 0));
    let password = "A".repeat(63);
    let payload = credentials_payload("Zorvinkel-Box", &password).unwrap();
    let request = Request {
        id: 7,
        operation: Operation::ApWrite,
    };
    let bytes = encode_request_payload(request, &payload).unwrap();
    assert_eq!(decode_request(&bytes).unwrap(), request);
    assert_eq!(request_payload(&bytes).unwrap(), payload);
    let read = read_credentials(&payload).unwrap();
    assert_eq!(
        (read.ssid.as_str(), read.password.as_str()),
        ("Zorvinkel-Box", password.as_str())
    );
    assert!(!format!("{read:?}").contains(&password));
    for (ssid, password) in [
        ("", "valid-password"),
        ("x", "short"),
        ("x", "ünicode-password"),
        ("x\n", "valid-password"),
        (&"s".repeat(33)[..], "valid-password"),
        ("x", &"p".repeat(64)[..]),
    ] {
        assert!(credentials_payload(ssid, password).is_err());
    }
    assert!(read_credentials(&payload[..payload.len() - 1]).is_err());
    let mut trailing = payload.clone();
    trailing.push(0);
    assert!(read_credentials(&trailing).is_err());
    for operation in [Operation::ApEnable, Operation::ApDisable] {
        let request = Request { id: 8, operation };
        let bytes = encode_request_payload(request, &id_payload(5).unwrap()).unwrap();
        assert_eq!(decode_request(&bytes).unwrap(), request);
        assert!(encode_request_payload(request, &[]).is_err());
    }
    let read = Request {
        id: 9,
        operation: Operation::ApRead,
    };
    assert_eq!(
        decode_request(&encode_request(read).unwrap()).unwrap(),
        read
    );
    assert!(encode_request_payload(read, &[1]).is_err());
    assert!(
        encode_request_payload(
            Request {
                id: 1,
                operation: Operation::Unknown(12)
            },
            &[]
        )
        .is_err()
    );
    for body in [
        ResponseBody::ApCredentials(None),
        ResponseBody::ApCredentials(Some(ApCredentials {
            ssid: "Box".into(),
            password: "valid-password".into(),
        })),
        ResponseBody::ApStored,
        ResponseBody::ApStarted(5),
    ] {
        let response = Response { id: 3, body };
        assert_eq!(
            decode_response(&encode_response(&response).unwrap()).unwrap(),
            response
        );
    }
}

#[test]
fn system_info_messages_are_typed_bounded_and_distinct_from_other_capabilities() {
    use zorvinkel_protocol::system::*;
    let others = [
        CAPABILITIES,
        zorvinkel_protocol::wifi::CAP_WIFI_TEMPORARY,
        zorvinkel_protocol::wifi::CAP_WIFI_PERSISTENCE,
        zorvinkel_protocol::power::CAP_POWER,
        zorvinkel_protocol::ap::CAP_ACCESS_POINT,
    ];
    assert!(others.iter().all(|bits| bits & CAP_SYSTEM_INFO == 0));
    let full = || SystemInfo {
        version: "0.1.0".into(),
        uptime_seconds: 93_784,
        load_milli: [120, 340, 280],
        memory_total: 485 << 20,
        memory_available: 310 << 20,
        storage_total: 14 << 30,
        storage_free: 9 << 30,
        temperature_milli: Some(47_800),
        cpu_mhz: Some(700),
        cpu_max_mhz: Some(1000),
        throttle_flags: Some(0x50005),
        os_name: Some("Raspbian GNU/Linux 10 (buster)".into()),
        kernel: Some("5.10.103+".into()),
        model: Some("Raspberry Pi Zero W Rev 1.1".into()),
        hostname: Some("Fotobox".into()),
        addresses: vec![
            NetworkAddress {
                interface: "wlan0".into(),
                address: "192.168.1.42".parse().unwrap(),
            },
            NetworkAddress {
                interface: "wlan0".into(),
                address: "2001:db8::5".parse().unwrap(),
            },
        ],
    };
    let sparse = SystemInfo {
        temperature_milli: Some(-5_000),
        cpu_mhz: None,
        cpu_max_mhz: None,
        throttle_flags: None,
        os_name: None,
        kernel: None,
        model: None,
        hostname: None,
        addresses: Vec::new(),
        ..full()
    };
    for value in [full(), sparse] {
        assert_eq!(decode(&encode(&value).unwrap()).unwrap(), value);
        let response = Response {
            id: 3,
            body: ResponseBody::SystemInfo(value),
        };
        assert_eq!(
            decode_response(&encode_response(&response).unwrap()).unwrap(),
            response
        );
    }
    let request = Request {
        id: 3,
        operation: Operation::SystemInfo,
    };
    assert_eq!(
        decode_request(&encode_request(request).unwrap()).unwrap(),
        request
    );
    assert!(encode_request_payload(request, &[1]).is_err());

    let valid = encode(&full()).unwrap();
    // Truncated and trailing bytes are rejected.
    assert!(decode(&valid[..valid.len() - 1]).is_err());
    assert!(decode(&[valid.clone(), vec![0]].concat()).is_err());
    // Impossible or oversized values are rejected.
    let invalid = [
        SystemInfo {
            memory_available: (485 << 20) + 1,
            ..full()
        },
        SystemInfo {
            storage_free: (14 << 30) + 1,
            ..full()
        },
        SystemInfo {
            version: String::new(),
            ..full()
        },
        SystemInfo {
            version: "has space".into(),
            ..full()
        },
        SystemInfo {
            version: "v".repeat(33),
            ..full()
        },
        SystemInfo {
            hostname: Some("x".repeat(65)),
            ..full()
        },
        SystemInfo {
            model: Some("line\nbreak".into()),
            ..full()
        },
        SystemInfo {
            os_name: Some(String::new()),
            ..full()
        },
        SystemInfo {
            addresses: vec![full().addresses[0].clone(); 9],
            ..full()
        },
        SystemInfo {
            addresses: vec![NetworkAddress {
                interface: "a-very-long-interface".into(),
                address: "10.0.0.1".parse().unwrap(),
            }],
            ..full()
        },
    ];
    for value in invalid {
        assert!(encode(&value).is_err(), "{value:?}");
    }
    // An unknown address family is rejected.
    let mut bad_family = encode(&SystemInfo {
        addresses: vec![full().addresses[0].clone()],
        ..full()
    })
    .unwrap();
    let family_index = bad_family.len() - 5;
    assert_eq!(bad_family[family_index], 4);
    bad_family[family_index] = 5;
    assert!(decode(&bad_family).is_err());
}

#[test]
fn service_list_messages_are_typed_bounded_and_distinct_from_other_capabilities() {
    use zorvinkel_protocol::services::*;
    let others = [
        CAPABILITIES,
        zorvinkel_protocol::wifi::CAP_WIFI_TEMPORARY,
        zorvinkel_protocol::wifi::CAP_WIFI_PERSISTENCE,
        zorvinkel_protocol::power::CAP_POWER,
        zorvinkel_protocol::ap::CAP_ACCESS_POINT,
        zorvinkel_protocol::system::CAP_SYSTEM_INFO,
        zorvinkel_protocol::ip::CAP_IP_REACHABILITY,
    ];
    assert!(others.iter().all(|bits| bits & CAP_SERVICES == 0));
    let plain = |name: &str| ServiceEntry {
        group: "Musik".into(),
        name: name.into(),
        version: None,
        state: ServiceState::Unknown,
        actions: Vec::new(),
        web: None,
    };
    let web = |label: Option<&str>, path: &str| WebLink {
        label: label.map(Into::into),
        scheme: WebScheme::Https,
        port: 6680,
        path: path.into(),
    };
    let full = ServiceEntry {
        version: Some("0.48.1".into()),
        state: ServiceState::Failed,
        actions: vec![ServiceAction::Start, ServiceAction::Restart],
        web: Some(web(Some("Iris"), "/iris/?a=b")),
        ..plain("Mopidy")
    };
    let unlabelled = ServiceEntry {
        web: Some(web(None, "/")),
        ..plain("Raspotify")
    };
    for state in [
        ServiceState::Active,
        ServiceState::Activating,
        ServiceState::Inactive,
    ] {
        let value = vec![
            full.clone(),
            unlabelled.clone(),
            ServiceEntry {
                state,
                ..plain("x")
            },
        ];
        assert_eq!(decode(&encode(&value).unwrap()).unwrap(), value);
    }
    assert_eq!(decode(&encode(&[]).unwrap()).unwrap(), Vec::new());
    let response = Response {
        id: 3,
        body: ResponseBody::Services(vec![full.clone()]),
    };
    assert_eq!(
        decode_response(&encode_response(&response).unwrap()).unwrap(),
        response
    );
    let request = Request {
        id: 3,
        operation: Operation::Services,
    };
    assert_eq!(
        decode_request(&encode_request(request).unwrap()).unwrap(),
        request
    );
    assert!(encode_request_payload(request, &[1]).is_err());

    // The largest list still fits one secured message.
    let largest = ServiceEntry {
        group: "g".repeat(MAX_NAME),
        name: "n".repeat(MAX_NAME),
        version: Some("v".repeat(MAX_VERSION)),
        state: ServiceState::Active,
        actions: vec![
            ServiceAction::Start,
            ServiceAction::Stop,
            ServiceAction::Restart,
        ],
        web: Some(web(
            Some(&"l".repeat(MAX_NAME)),
            &format!("/{}", "p".repeat(MAX_PATH - 1)),
        )),
    };
    let maximum = ResponseBody::Services(vec![largest; MAX_SERVICES]);
    assert!(
        encode_response(&Response {
            id: 1,
            body: maximum
        })
        .is_ok()
    );

    let valid = encode(std::slice::from_ref(&full)).unwrap();
    assert!(decode(&valid[..valid.len() - 1]).is_err());
    assert!(decode(&[valid.clone(), vec![0]].concat()).is_err());
    let invalid = [
        vec![plain("x"); MAX_SERVICES + 1],
        vec![plain("")],
        vec![plain(&"n".repeat(MAX_NAME + 1))],
        vec![plain("line\nbreak")],
        vec![ServiceEntry {
            group: String::new(),
            ..plain("x")
        }],
        vec![ServiceEntry {
            version: Some(String::new()),
            ..plain("x")
        }],
        vec![ServiceEntry {
            version: Some("v".repeat(MAX_VERSION + 1)),
            ..plain("x")
        }],
        vec![ServiceEntry {
            web: Some(web(None, "no-slash")),
            ..plain("x")
        }],
        vec![ServiceEntry {
            web: Some(web(None, "/a b")),
            ..plain("x")
        }],
        vec![ServiceEntry {
            web: Some(web(None, "/ä")),
            ..plain("x")
        }],
        vec![ServiceEntry {
            web: Some(web(None, &format!("/{}", "p".repeat(MAX_PATH)))),
            ..plain("x")
        }],
        vec![ServiceEntry {
            web: Some(web(Some(""), "/")),
            ..plain("x")
        }],
        vec![ServiceEntry {
            web: Some(WebLink {
                port: 0,
                ..web(None, "/")
            }),
            ..plain("x")
        }],
    ];
    for value in invalid {
        assert!(encode(&value).is_err(), "{value:?}");
    }
    let twice = ServiceEntry {
        actions: vec![ServiceAction::Stop, ServiceAction::Stop],
        ..plain("x")
    };
    assert!(encode(&[twice]).is_err());
    // Unknown state, action and scheme codes are rejected.
    let mut bytes = encode(&[plain("x")]).unwrap();
    let state_index = bytes.len() - 3;
    bytes[state_index] = 9;
    assert!(decode(&bytes).is_err());
    let mut bytes = encode(&[plain("x")]).unwrap();
    let mask_index = bytes.len() - 2;
    bytes[mask_index] = 8;
    assert!(decode(&bytes).is_err());
    let mut bytes = encode(&[plain("x")]).unwrap();
    let scheme_index = bytes.len() - 1;
    bytes[scheme_index] = 3;
    assert!(decode(&bytes).is_err());
}

#[test]
fn service_action_messages_are_typed_and_bounded() {
    use zorvinkel_protocol::services::*;
    assert_eq!(
        [
            CAP_SERVICE_ACTIONS & CAP_SERVICES,
            CAP_SERVICE_ACTIONS & zorvinkel_protocol::ip::CAP_IP_REACHABILITY
        ],
        [0, 0]
    );
    for action in [
        ServiceAction::Start,
        ServiceAction::Stop,
        ServiceAction::Restart,
    ] {
        let payload = action_payload(action, "Raspotify").unwrap();
        assert_eq!(
            read_action(&payload).unwrap(),
            (action, "Raspotify".to_owned())
        );
        let request = Request {
            id: 4,
            operation: Operation::ServiceAction,
        };
        let bytes = encode_request_payload(request, &payload).unwrap();
        assert_eq!(decode_request(&bytes).unwrap(), request);
        assert_eq!(request_payload(&bytes).unwrap(), payload);
        let response = Response {
            id: 4,
            body: ResponseBody::ServiceActionAccepted(action, "Raspotify".into()),
        };
        assert_eq!(
            decode_response(&encode_response(&response).unwrap()).unwrap(),
            response
        );
    }
    let request = Request {
        id: 4,
        operation: Operation::ServiceAction,
    };
    // Payload checks happen before dispatch.
    for invalid in [
        vec![],
        vec![1],
        vec![0, 1, b'x'],
        vec![4, 1, b'x'],
        vec![1, 0],
        vec![1, 1, b'x', 0],
        vec![1, 2, b'a', 0x07],
        [vec![1, 33], vec![b'n'; 33]].concat(),
    ] {
        assert!(read_action(&invalid).is_err(), "{invalid:?}");
        assert!(
            encode_request_payload(request, &invalid).is_err(),
            "{invalid:?}"
        );
    }
    assert!(action_payload(ServiceAction::Stop, "").is_err());
    assert!(action_payload(ServiceAction::Stop, &"n".repeat(MAX_NAME + 1)).is_err());
}

#[test]
fn correlation_messages_round_trip_and_reject_wrong_lengths() {
    use zorvinkel_protocol::ip::CorrelationToken;
    let token = CorrelationToken([7; 32]);
    let request =
        |operation, payload: &[u8]| encode_request_payload(Request { id: 1, operation }, payload);
    assert!(request(Operation::CorrelationIssue, &[]).is_ok());
    assert!(request(Operation::CorrelationIssue, &[1]).is_err());
    let redeem = request(Operation::CorrelationRedeem, &token.0).unwrap();
    assert_eq!(
        decode_request(&redeem).unwrap().operation,
        Operation::CorrelationRedeem
    );
    assert!(request(Operation::CorrelationRedeem, &[7; 31]).is_err());
    for body in [ResponseBody::Correlation(token), ResponseBody::Correlated] {
        let response = Response { id: 1, body };
        assert_eq!(
            decode_response(&encode_response(&response).unwrap()).unwrap(),
            response
        );
    }
}

#[test]
fn restart_policy_messages_round_trip_and_reject_malformed_input() {
    use zorvinkel_protocol::restart::*;
    let request =
        |operation, payload: &[u8]| encode_request_payload(Request { id: 1, operation }, payload);
    assert!(request(Operation::RestartPolicyRead, &[]).is_ok());
    assert!(request(Operation::RestartPolicyRead, &[0]).is_err());
    for change in [RestartChange::LastAccepted, RestartChange::CurrentAsDefault] {
        let payload = set_payload(7, change).unwrap();
        let bytes = request(Operation::RestartPolicySet, &payload).unwrap();
        assert_eq!(
            decode_request(&bytes).unwrap().operation,
            Operation::RestartPolicySet
        );
        let parsed = read_set(&payload).unwrap();
        assert_eq!((parsed.id, parsed.change), (7, change));
    }
    assert!(set_payload(0, RestartChange::LastAccepted).is_err());
    for invalid in [
        vec![],
        vec![0; 8],
        vec![1, 0, 0, 0, 0, 0, 0, 0, 2],
        vec![0; 9],
    ] {
        assert!(read_set(&invalid).is_err(), "{invalid:?}");
        assert!(
            request(Operation::RestartPolicySet, &invalid).is_err(),
            "{invalid:?}"
        );
    }
    for body in [
        ResponseBody::RestartPolicy(RestartState::LastAccepted),
        ResponseBody::RestartPolicy(RestartState::DefaultWifi),
        ResponseBody::RestartPolicy(RestartState::DefaultAccessPoint),
        ResponseBody::RestartPolicyStarted(9),
    ] {
        let response = Response { id: 1, body };
        assert_eq!(
            decode_response(&encode_response(&response).unwrap()).unwrap(),
            response
        );
    }
    assert!(read_state(&[3]).is_err());
    assert!(
        encode_request_payload(
            Request {
                id: 1,
                operation: Operation::Unknown(19)
            },
            &[]
        )
        .is_err()
    );
}
