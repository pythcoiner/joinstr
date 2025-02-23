#include "joinstr.h"

auto errorToString(JoinstrError e) -> std::string {
    switch (e) {
    case JoinstrError::None:
        return "None";
    case JoinstrError::Tokio:
        return "Tokio";
    case JoinstrError::CastString:
        return "CastString";
    case JoinstrError::JsonError:
        return "Json";
    case JoinstrError::CString:
        return "CString";
    case JoinstrError::ListPools:
        return "ListPools";
    case ListCoins:
        return "ListCoins";
    case InitiateConjoin:
        return "InitiateCoinjoin";
    case SerdeJson:
        return "SerdeJson";
    case PoolConfig:
        return "PoolConfig";
    case PeerConfig:
        return "PeerConfig";
    }
    return "Unknown";
}
