#pragma once

#include <cstdint>
#include <stdint.h>
#include <string>
using f64_t = double;

extern "C" {

enum Network { // NOLINT(performance-enum-size)
    Bitcoin = 0,
    Testnet,
    Signet,
    Regtest,
};

enum JoinstrError { // NOLINT(performance-enum-size)
    None = 0,
    Tokio,
    CastString,
    JsonError,
    CString,
    ListPools,
    ListCoins,
    InitiateConjoin,
    SerdeJson,
    PoolConfig,
    PeerConfig,
};

struct PoolConfig {
    f64_t denomination;
    uint32_t fee;
    uint64_t max_duration;
    uint8_t peers;
    Network network;
};

struct PeerConfig {
    const char* electrum_address;
    uint16_t electrum_port;
    const char* mnemonics;
    const char* input;
    const char* output;
    const char* relay;
};

struct Pools {
    const char* pools;
    JoinstrError error;
};

struct Coins {
    const char* coins;
    JoinstrError error;
};

struct Txid {
    const char* txid;
    JoinstrError error;
};

auto list_pools( // NOLINT(readability-identifier-naming)
    uint64_t back, 
    uint64_t timeout, 
    const char* relay
) -> Pools;

auto list_coins( // NOLINT(readability-identifier-naming)
    const char* mnemonics,
    const char* addr,
    uint16_t port,
    Network network,
    uint32_t index_min,
    uint32_t index_max
) -> Coins;

auto initiate_coinjoin( // NOLINT(readability-identifier-naming)
    struct PoolConfig config, 
    struct PeerConfig peer
) -> Txid;

auto join_coinjoin( // NOLINT(readability-identifier-naming)
    const char* pool,
    struct PeerConfig peer
) -> Txid;

}

auto errorToString(JoinstrError e) -> std::string;
