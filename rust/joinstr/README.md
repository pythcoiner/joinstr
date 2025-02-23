# Disclaimer

This library is at (very) experimental stage, we do not advice to use it on mainnet.
There is still some minor differences at the protocol level between this  implementation
and the python & kotlin implementations, but we should fix it soon.

# Sponsorship

[R&D Sponsored By Bull Bitcoin](https://www.bullbitcoin.com/)

# Joinstr protocol
 - [Joinstr website](https://joinstr.xyz/)
 - [Kotlin implementation](https://gitlab.com/invincible-privacy/joinstr-kmp)
 - [Python implementation](https://gitlab.com/invincible-privacy/joinstr)

# Transaction Inputs/Outputs types

As of now, we have only implemented the protocol for using **Segwitv0** inputs & outputs.

# VPN/Tor

For now there is no plan to implement VPN or Tor support in this lib, as it's expected 
to be handled at consumer or OS level.

# Build

Install Rust toolchain ([see here](https://www.rust-lang.org/tools/install))

and run this from this repo:

```shell
cargo run --release
```

# Run the tests
```shell
cargo tests
```

# Usage

## Run a standalone coordinator

```rust
    let mut coordinator = Joinstr::new_initiator(
        Keys::generate(),
        "wss://relay.nostr".into(),
        ("127.0.0.1", 2121),
        Network::Regtest,
        "initiator",
    )
    .unwrap()
    .denomination(0.01)
    .unwrap()
    .fee(10)
    .unwrap()
    .simple_timeout(now() + 3 * 60 * 60)
    .unwrap()
    .min_peers(5)
    .unwrap();
    coordinator
        .start_coinjoin(None, Option::<&WpkhHotSigner>::None)
        
        .unwrap();
```

## Initiate a pool

```rust 
    // create an electrum client
    let client = Client::new_local("127.0.0.1", 2121).unwrap();

    // create the signer
    let mnemonic =
        "define jealous drill wrap item shallow chest balcony domain dignity runway year";
    let mut signer = WpkhHotSigner::new_from_mnemonics(Network::Regtest, mnemonic).unwrap();
    signer.set_client(client);

    // fetch the coin you want to add to the pool
    let coins = signer
        .get_coins_at(CoinPath {
            depth: 0,
            index: Some(0),
        })
        .unwrap();
    assert_eq!(coins, 1);
    let coin = signer.list_coins().into_iter().next().unwrap();

    // generate the output address
    let address = signer
        .address_at(&CoinPath {
            depth: 0,
            index: Some(100),
        })
        .unwrap()
        .as_unchecked()
        .clone();

    // create a peer that will also be a coordinator
    let mut peer = Joinstr::new_peer(
        "wss://relay.nostr".into(),
        &pool,
        coin.1,
        address,
        Network::Regtest,
        "peer_a",
    )
    
    .unwrap()
    .denomination(0.01)
    .unwrap()
    .fee(10)
    .unwrap()
    .simple_timeout(now() + 3 * 60 * 60)
    .unwrap()
    .min_peers(5)
    .unwrap();

    // try to run the coinjoin
    peer.start_coinjoin(None, Some(&signer))
        .unwrap();

```

## Join a pool as peer

```rust 
    // create a nostr client and listen for pool notification
    let mut pool_listener = NostrClient::new("pool_listener")
        .relay("wss://relay.nostr".into())
        .unwrap()
        .keys(Keys::generate())
        .unwrap();
    pool_listener.connect_nostr().unwrap();
    // subscribe to pool notifications that have been initiated 2 hours back in time
    pool_listener.subscribe_pools(2 * 60 * 60).unwrap();

    // wait to receive notifications
    sleep(Duration::from_millis(3000));

    // list received notifications
    let mut pools = Vec::new();

    while let Some(pool) = pool_listener.receive_pool_notification().unwrap() {
        pools.push(pool)
    }

    // select the pool you want to join (like by pool denomination and network config)
    let pool = pools.into_iter().next().unwrap();

    // create an electrum client
    let client = Client::new_local("127.0.0.1", 2121).unwrap();

    // create the signer
    let mnemonic =
        "define jealous drill wrap item shallow chest balcony domain dignity runway year";
    let mut signer = WpkhHotSigner::new_from_mnemonics(Network::Regtest, mnemonic).unwrap();
    signer.set_client(client);

    // fetch the coin you want to add to the pool
    let coins = signer
        .get_coins_at(CoinPath {
            depth: 0,
            index: Some(0),
        })
        .unwrap();
    assert_eq!(coins, 1);
    let coin = signer.list_coins().into_iter().next().unwrap();

    // generate the output address
    let address = signer
        .address_at(&CoinPath {
            depth: 0,
            index: Some(100),
        })
        .unwrap()
        .as_unchecked()
        .clone();

    // create a peer that will also be a coordinator
    let mut peer = Joinstr::new_peer(
        "wss://relay.nostr".into(),
        &pool,
        coin.1,
        address,
        Network::Regtest,
        "peer_a",
    )
    .unwrap();

    // try to run the coinjoin
    peer.start_coinjoin(Some(pool), Some(&signer))
        .unwrap();

```
    
