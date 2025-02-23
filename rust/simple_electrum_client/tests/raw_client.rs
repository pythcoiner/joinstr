use std::{
    collections::HashMap,
    env,
    path::PathBuf,
    str::FromStr,
    thread,
    time::{Duration, Instant},
};

use electrsd::{
    bitcoind::{bitcoincore_rpc::RpcApi, BitcoinD, P2P},
    ElectrsD,
};
use miniscript::bitcoin::{hex::FromHex, OutPoint, Script};
use serde_json::Value;
use simple_electrum_client::{
    electrum::{request::Request, response::*},
    raw_client::Client,
};

fn bootstrap_electrs() -> (String, u16, ElectrsD, BitcoinD) {
    let mut cwd: PathBuf = env::current_dir().expect("Failed to get current directory");
    cwd.push("tests");

    let mut electrs_path = cwd.clone();
    electrs_path.push("bin");
    electrs_path.push("electrs_0_9_11");

    let mut bitcoind_path = cwd.clone();
    bitcoind_path.push("bin");
    bitcoind_path.push("bitcoind_25_2");

    let mut conf = electrsd::bitcoind::Conf::default();
    conf.p2p = P2P::Yes;
    let bitcoind = BitcoinD::with_conf(bitcoind_path, &conf).unwrap();

    let electrsd_conf = electrsd::Conf::default();
    // electrsd_conf.view_stderr = true;
    let electrsd = ElectrsD::with_conf(electrs_path, &bitcoind, &electrsd_conf).unwrap();
    let (url, port) = electrsd.electrum_url.split_once(':').unwrap();
    let port = port.parse::<u16>().unwrap();
    (url.into(), port, electrsd, bitcoind)
}

fn tcp_client() -> (Client, ElectrsD, BitcoinD) {
    let (url, port, electrs, bitcoind) = bootstrap_electrs();
    let mut c = Client::new().tcp(&url, port);
    c.connect();
    (c, electrs, bitcoind)
}

fn env_var(arg: &str) -> Option<String> {
    if let Ok(value) = env::var(arg) {
        Some(value)
    } else {
        None
    }
}

fn ssl_local_address() -> Option<String> {
    env_var("SSL_LOCAL_ADDRESS")
}
fn ssl_acinq() -> String {
    "electrum.acinq.co:50002".into()
}

fn split_url(url: String) -> (String, u16) {
    let (url, port) = url.rsplit_once(':').unwrap();
    let port = port.parse::<u16>().unwrap();
    (url.to_string(), port)
}

fn acinq_client() -> Client {
    let (url, port) = split_url(ssl_acinq());
    Client::new().ssl(&url, port)
}

#[test]
fn ping() {
    let (mut client, _electrs, _bitcoind) = tcp_client();

    // blocking recv
    client.send_str("ping");
    let _ = client.recv_str().unwrap();

    // non blocking recv
    client.send_str("ping");
    thread::sleep(Duration::from_secs(1));
    assert!(client.try_recv_str().unwrap().is_some());
    assert!(client.try_recv_str().unwrap().is_none());

    client.close().unwrap();
}

#[test]
fn ping_request() {
    let (mut client, _electrs, _bitcoind) = tcp_client();

    let request = Request::ping();
    client.send(&request);
    let mut index = HashMap::new();
    index.insert(request.id, request);

    let response = &client.recv(&index).unwrap()[0];

    if let Response::Ping(_) = response {
        //
    } else {
        panic!("wrong response")
    }

    client.close().unwrap();
}

#[test]
// NOTE: SSL_LOCAL_ADDRESS should be specified in order
// to enable this test
fn ssl_client_wo_certificate() {
    if let Some(address) = ssl_local_address() {
        let (url, port) = split_url(address);
        let mut client = Client::new().ssl(&url, port);
        assert!(client.try_connect().is_err());
        let mut client = client.verif_certificate(false);
        client.connect();

        // blocking recv
        client.send_str("ping");
        let _ = client.recv_str().unwrap();

        // non blocking recv
        client.send_str("ping");
        thread::sleep(Duration::from_secs(1));
        assert!(client.try_recv_str().unwrap().is_some());
        assert!(client.try_recv_str().unwrap().is_none());

        client.close().unwrap();
    }
}

#[test]
fn ssl_client_with_certificate() {
    let (url, port) = split_url(ssl_acinq());
    let mut client = Client::new_ssl(&url, port);
    client.connect();
    client.send_str("ping");
    let _ = client.recv_str().unwrap();
    client.close().unwrap();
}

#[test]
fn ssl_maybe() {
    let (url, port) = split_url(ssl_acinq());
    let mut client = Client::new_ssl_maybe(&url, port, true);
    client.connect();
    client.send_str("ping");
    let _ = client.recv_str().unwrap();
    client.close().unwrap();
}

#[test]
fn tcp_clone() {
    let (mut client, _e, _b) = tcp_client();
    let mut cloned = client.clone();

    client.send_str("ping");
    cloned.recv_str().unwrap();
}

fn timeout_template(url: &str, port: u16, ssl: bool) {
    let mut client = Client::new_ssl_maybe(url, port, ssl)
        .verif_certificate(false)
        .read_timeout(Some(Duration::from_millis(100)));
    client.connect();
    let start = Instant::now();
    let resp = client.recv_str();
    let duration = (Instant::now() - start).as_millis();
    assert!(duration > 100);
    assert_eq!(
        format!("{resp:?}"),
        r#"Err(TcpStream(Os { code: 11, kind: WouldBlock, message: "Resource temporarily unavailable" }))"#
    );

    let mut client = Client::new_ssl_maybe(url, port, ssl).verif_certificate(false);
    client.connect();
    client
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let start = Instant::now();
    let resp = client.recv_str();
    let duration = (Instant::now() - start).as_millis();
    assert!(duration > 500);
    assert!(duration < 600);
    assert_eq!(
        format!("{resp:?}"),
        r#"Err(TcpStream(Os { code: 11, kind: WouldBlock, message: "Resource temporarily unavailable" }))"#
    );
}

// TODO: disable timeouts in CI

// #[test]
// fn timeout_tcp() {
//     let (url, port, _e, _b) = bootstrap_electrs();
//     timeout_template(&url, port, false);
// }

#[test]
// NOTE: SSL_LOCAL_ADDRESS should be specified in order
// to enable this test
fn timeout_ssl() {
    if let Some(address) = ssl_local_address() {
        let (url, port) = split_url(address);
        timeout_template(&url, port, true);
    }
}

#[test]
fn banner() {
    let request = Request::banner();
    let (mut client, _e, _b) = tcp_client();
    client.send(&request);
    let mut index = HashMap::new();
    index.insert(request.id, request);
    matches!(
        client.recv(&index).unwrap()[0],
        Response::Banner(BannerResponse { id: 0, .. })
    );
}

#[test]
fn block_header() {
    let (mut client, _e, bitcoind) = tcp_client();

    // generate 20 blocks
    let node_address = bitcoind.client.call::<Value>("getnewaddress", &[]).unwrap();
    bitcoind
        .client
        .call::<Value>("generatetoaddress", &[20.into(), node_address])
        .unwrap();

    // wait for electrs to update
    thread::sleep(Duration::from_millis(1500));

    let request = Request::subscribe_headers();
    client.send(&request);
    let mut index = HashMap::new();
    index.insert(request.id, request.clone());

    // We get the chain tip w/ a Request.subscribe_header()
    let mut responses = client.recv(&index).unwrap();
    // NOTE: there is no method to unsubscribe so during the test
    // we can receive an unintended header notification

    // waiting for height 20
    let response: Response;
    'l: loop {
        for ref r in responses {
            match r {
                r @ Response::HeaderNotif(HeaderNotification::Single(SingleHeaderNotif {
                    header: Header { height, .. },
                    ..
                })) => {
                    if *height > 19 {
                        response = r.clone();
                        break 'l;
                    }
                }
                Response::BatchHeaderNotif(BatchHeaderNotif { headers, .. }) => {
                    for header in headers {
                        let Header { height, .. } = &header;
                        if *height > 19 {
                            // NOTE: we convert it into a HeaderNotification single
                            // TODO: maybe implement a method for fetch id from requests index?
                            response = Response::HeaderNotif(HeaderNotification::Single(
                                SingleHeaderNotif {
                                    id: 0,
                                    header: header.clone(),
                                },
                            ));
                            break 'l;
                        }
                    }
                }
                r => {
                    panic!("wrong response: {:?}", r);
                }
            }
        }
        thread::sleep(Duration::from_millis(100));
        responses = client.recv(&index).unwrap();
    }

    if let Response::HeaderNotif(HeaderNotification::Single(SingleHeaderNotif {
        header: Header { height, .. },
        ..
    })) = &response
    {
        let height = height - 10;

        // We can now not get a single header
        let request = Request::header(height);
        client.send(&request);
        index.insert(request.id, request);
        let mut responses = client.recv(&index).unwrap();
        // NOTE: here we can receive some HeaderNotification::Batch at any time
        // so we need filter out them
        let response: Response;
        'l: loop {
            for r in responses {
                match r {
                    r @ Response::Header(_) => {
                        response = r;
                        break 'l;
                    }
                    Response::BatchHeaderNotif(_) => continue,
                    _ => panic!(" wrong response"),
                }
            }
            responses = client.recv(&index).unwrap();
        }

        if let Response::Header(HeaderResponse { id, raw_header }) = response {
            assert_eq!(id, 0);
            assert!(!raw_header.is_empty())
        } else {
            panic!("wrong response")
        }

        // Now get several headers
        let request = Request::headers(height, 5);
        client.send(&request);
        index.insert(request.id, request);
        let response = client.recv(&index).unwrap();
        // TODO: handle unintended notification
        if let Response::Headers(HeadersResponse {
            id,
            headers:
                Headers {
                    count,
                    raw_headers,
                    max,
                },
        }) = &response[0]
        {
            assert_eq!(*id, 0);
            assert_eq!(*count, 5);
            assert!(*max > 1);
            assert!(!raw_headers.is_empty())
        } else {
            panic!("wrong response")
        }
    } else {
        unreachable!()
    }
}

#[test]
fn test_version() {
    let (mut client, _e, _b) = tcp_client();

    let request = Request::version("smart".into(), "1.4".into());
    client.send(&request);
    let mut index = HashMap::new();
    index.insert(request.id, request.clone());
    let response = client.recv(&index).unwrap();
    if let Response::Version(VersionResponse { id, .. }) = response[0] {
        assert_eq!(id, 0);
    } else {
        panic!("wrong response");
    }
}

#[test]
// TODO: use tcp_client() instead
fn tx_get() {
    let mut client = acinq_client();
    client.connect();

    let raw_outpoint = "e03a9a4b5c557f6ee3400a29ff1475d1df73e9cddb48c2391abdc391d8c1504a:0";
    let outpoint = OutPoint::from_str(raw_outpoint).unwrap();

    // get raw tx
    let request = Request::tx_get(outpoint.txid);
    client.send(&request);
    let mut index = HashMap::new();
    index.insert(request.id, request.clone());
    let response = client.recv(&index).unwrap();
    if let Response::TxGet(TxGetResponse {
        id,
        result: TxGetResult::Raw(raw_tx),
    }) = &response[0]
    {
        assert_eq!(*id, 0);
        assert!(!raw_tx.is_empty());
    } else {
        panic!("wrong response")
    }

    // get verbose tx
    let request = Request::tx_get_verbose(outpoint.txid);
    client.send(&request);
    let mut index = HashMap::new();
    index.insert(request.id, request.clone());
    let response = client.recv(&index).unwrap();
    if let Response::TxGet(TxGetResponse {
        id,
        result: TxGetResult::Verbose(tx),
    }) = &response[0]
    {
        assert_eq!(*id, 0);
        assert!(!tx.raw_tx.is_empty())
    } else {
        panic!("wrong response")
    }
}

// NOTE: Unsubscribe not supported on electrs 0_9_11

// #[test]
// fn sh_subscribe_unsubscribe() {
//     let (mut client, _e, _b) = tcp_client();
//
//     let mut index = HashMap::new();
//
//     let script = Script::from_bytes(&[0x00]);
//
//     // unsubscribe w/o subscription we expect result==false
//     let request = Request::unsubscribe_sh(script);
//     client.send(&request);
//     index.insert(request.id, request);
//     let response = &client.recv(&index).unwrap()[0];
//     if let Response::SHUnsubscribe(SHUnsubscribeResponse { id, result }) = response {
//         assert_eq!(*id, 0);
//         assert!(!(*result));
//     } else {
//         panic!("wrong_response")
//     }
//
//     // subscribe
//     let request = Request::subscribe_sh(script).id(1);
//     client.send(&request);
//     index.insert(request.id, request);
//     let response = &client.recv(&index).unwrap()[0];
//     if let Response::SHSubscribe(SHSubscribeResponse { id, result }) = response {
//         assert_eq!(*id, 1);
//         assert_eq!(*result, None);
//     } else {
//         panic!("wrong_response")
//     }
//
//     // unsubscribe w/ subscription we expect result==true
//     let request = Request::unsubscribe_sh(script).id(2);
//     client.send(&request);
//     index.insert(request.id, request);
//     let response = &client.recv(&index).unwrap()[0];
//     if let Response::SHUnsubscribe(SHUnsubscribeResponse { id, result }) = response {
//         assert_eq!(*id, 2);
//         assert!((*result));
//     } else {
//         panic!("wrong_response")
//     }
// }

#[test]
// TODO: use tcp_client() instead
fn sh_get_balance() {
    let mut client = acinq_client();
    client.connect();

    let raw_script = Vec::from_hex("0014992f8cc4f6d284acac5f603e233592b566c04b2a").unwrap();
    let script = Script::from_bytes(raw_script.as_slice());

    let mut index = HashMap::new();
    let request = Request::sh_get_balance(script);
    client.send(&request);
    index.insert(request.id, request);
    let response = &client.recv(&index).unwrap()[0];
    if let Response::SHGetBalance(_) = response {
    } else {
        panic!("wrong response")
    }
}

#[test]
// TODO: use tcp_client() instead
fn sh_get_history() {
    let mut client = acinq_client();
    client.connect();

    let raw_script = Vec::from_hex("0014992f8cc4f6d284acac5f603e233592b566c04b2a").unwrap();
    let script = Script::from_bytes(raw_script.as_slice());

    let mut index = HashMap::new();
    let request = Request::sh_get_history(script);
    client.send(&request);
    index.insert(request.id, request);
    let response = &client.recv(&index).unwrap()[0];
    if let Response::SHGetHistory(_) = response {
    } else {
        panic!("wrong response")
    }
}

// NOTE: not supported by electrs
// #[test]
// fn sh_get_mempool() {
//     let mut client = tcp_client();
//     client.connect();
//
//     let script = Script::from_bytes(&[0x00]);
//
//     let mut index = HashMap::new();
//     let request = Request::sh_get_mempool(script);
//     client.send(&request);
//     index.insert(request.id, request);
//     println!("get_mempool: {}", client.recv_str().unwrap());
//     // let response = client.recv(&index).unwrap()[0];
// }

#[test]
// TODO: use tcp_client() instead
fn sh_list_unspent() {
    let mut client = acinq_client();
    client.connect();

    let raw_script = Vec::from_hex("0014992f8cc4f6d284acac5f603e233592b566c04b2a").unwrap();
    let script = Script::from_bytes(raw_script.as_slice());

    let mut index = HashMap::new();
    let request = Request::sh_list_unspent(script);
    client.send(&request);
    index.insert(request.id, request);
    let response = &client.recv(&index).unwrap()[0];
    if let Response::SHListUnspent(_) = response {
    } else {
        panic!("wrong response")
    }
}

#[test]
fn features() {
    let (mut client, _e, _b) = tcp_client();

    let mut index = HashMap::new();
    let request = Request::features();
    client.send(&request);
    index.insert(request.id, request);
    let response = &client.recv(&index).unwrap()[0];
    if let Response::Features(_) = response {
        //
    } else {
        panic!("wrong response")
    }
}

#[test]
fn donation() {
    let (mut client, _e, _b) = tcp_client();

    let mut index = HashMap::new();
    let request = Request::donation();
    client.send(&request);
    index.insert(request.id, request);
    let response = &client.recv(&index).unwrap()[0];
    if let Response::Donation(_) = response {
        //
    } else {
        panic!("wrong response")
    }
}

#[test]
fn estimate_fee() {
    let mut client = acinq_client();
    client.connect();

    let mut index = HashMap::new();
    let request = Request::estimate_fee(10);
    client.send(&request);
    index.insert(request.id, request);
    let response = &client.recv(&index).unwrap()[0];
    if let Response::EstimateFee(_) = response {
        //
    } else {
        panic!("wrong response")
    }
}

#[test]
fn fee_histogram() {
    let mut client = acinq_client();
    client.connect();

    let mut index = HashMap::new();
    let request = Request::get_fee_histogram();
    client.send(&request);
    index.insert(request.id, request);
    let response = &client.recv(&index).unwrap()[0];
    if let Response::FeeHistogram(_) = response {
        //
    } else {
        panic!("wrong response")
    }
}

#[test]
fn relay_fee() {
    let mut client = acinq_client();
    client.connect();

    let mut index = HashMap::new();
    let request = Request::relay_fee();
    client.send(&request);
    index.insert(request.id, request);
    // println!("{}", client.recv_str().unwrap());
    let response = &client.recv(&index).unwrap()[0];
    if let Response::RelayFee(_) = response {
        //
    } else {
        panic!("wrong response")
    }
}

#[test]
// TODO: use tcp_client() instead
fn tx_get_merkle() {
    let mut client = acinq_client();
    client.connect();

    let raw_outpoint = "e03a9a4b5c557f6ee3400a29ff1475d1df73e9cddb48c2391abdc391d8c1504a:0";
    let outpoint = OutPoint::from_str(raw_outpoint).unwrap();

    let mut index = HashMap::new();
    let request = Request::tx_get_merkle(outpoint.txid, 200_000);
    client.send(&request);
    index.insert(request.id, request);
    // println!("{}", client.recv_str().unwrap());
    let response = &client.recv(&index).unwrap()[0];
    if let Response::TxGetMerkle(_) = response {
        //
    } else {
        panic!("wrong response")
    }
}

#[test]
// TODO: use tcp_client() instead
fn tx_from_position() {
    let mut client = acinq_client();
    client.connect();

    let mut index = HashMap::new();
    let request = Request::tx_from_pos(200_000, 3, false);
    client.send(&request);
    index.insert(request.id, request);
    let response = &client.recv(&index).unwrap()[0];
    if let Response::TxFromposition(_) = response {
        //
    } else {
        panic!("wrong response")
    }

    let request = Request::tx_from_pos(300_000, 125, true).id(1);
    client.send(&request);
    index.insert(request.id, request);
    let response = &client.recv(&index).unwrap()[0];
    if let Response::TxFromposition(_) = response {
        //
    } else {
        panic!("wrong response")
    }
}
