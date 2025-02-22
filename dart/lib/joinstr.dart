import 'dart:ffi';
import 'dart:io';
import 'package:ffi/ffi.dart';

/// Enums mapping
class Network {
  static const int Bitcoin = 0;
  static const int Testnet = 1;
  static const int Signet = 2;
  static const int Regtest = 3;
}

class JoinstrError {
  static const int None = 0;
  static const int Tokio = 1;
  static const int CastString = 2;
  static const int JsonError = 3;
  static const int CString = 4;
  static const int ListPools = 5;
  static const int ListCoins = 6;
  static const int InitiateConjoin = 7;
  static const int SerdeJson = 8;
  static const int PoolConfig = 9;
  static const int PeerConfig = 10;
}

/// Structs mapping
base class PoolConfig extends Struct {
  @Double()
  external double denomination;

  @Uint32()
  external int fee;

  @Uint64()
  external int maxDuration;

  @Uint8()
  external int peers;

  @Int32()
  external int network;
}

base class PeerConfig extends Struct {
  external Pointer<Utf8> electrumAddress;
  @Uint16()
  external int electrumPort;
  external Pointer<Utf8> mnemonics;
  external Pointer<Utf8> input;
  external Pointer<Utf8> output;
  external Pointer<Utf8> relay;
}

base class Pools extends Struct {
  external Pointer<Utf8> pools;
  @Int32()
  external int error;
}

base class Coins extends Struct {
  external Pointer<Utf8> coins;
  @Int32()
  external int error;
}

base class Txid extends Struct {
  external Pointer<Utf8> txid;
  @Int32()
  external int error;
}

/// FFI Binding Loader
class JoinstrBindings {
  static late final DynamicLibrary _lib;

  static void init() {
    if (Platform.isIOS) {
      // For static libraries (.a), use process()
      _lib = DynamicLibrary.process();
    } else if (Platform.isAndroid) {
      _lib = DynamicLibrary.open('libjoinstr.so');
    } else if (Platform.isLinux || Platform.isMacOS) {
      _lib = DynamicLibrary.open('libjoinstr.dylib');
    } else {
      throw UnsupportedError('Unsupported platform');
    }
  }

  static late final Pools Function(int, int, Pointer<Utf8>) listPools = _lib
      .lookupFunction<
        Pools Function(Uint64, Uint64, Pointer<Utf8>),
        Pools Function(int, int, Pointer<Utf8>)
      >('list_pools');

  static late final Coins Function(
    Pointer<Utf8>,
    Pointer<Utf8>,
    int,
    int,
    int,
    int,
  )
  listCoins = _lib.lookupFunction<
    Coins Function(Pointer<Utf8>, Pointer<Utf8>, Uint16, Int32, Uint32, Uint32),
    Coins Function(Pointer<Utf8>, Pointer<Utf8>, int, int, int, int)
  >('list_coins');

  static late final Txid Function(PoolConfig, PeerConfig) initiateCoinjoin =
      _lib.lookupFunction<
        Txid Function(PoolConfig, PeerConfig),
        Txid Function(PoolConfig, PeerConfig)
      >('initiate_coinjoin');

  static late final Txid Function(Pointer<Utf8>, PeerConfig) joinCoinjoin = _lib
      .lookupFunction<
        Txid Function(Pointer<Utf8>, PeerConfig),
        Txid Function(Pointer<Utf8>, PeerConfig)
      >('join_coinjoin');
}
