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

# Project organisation:

The rust library can be found [here](./rust/joinstr/README.md).

Experimentals bindings can be generated for C/C++/Dart by running `just binding` (you need to have [`just`](https://github.com/casey/just) installed) or directly the binding [script](./contrib/bindings.sh)

If no error during binding generation, bindings will be available at:
  - `rust/include/c` for the C headers
  - `rust/include/cpp` for the C++ headers
  - `dart/lib` for dart bindings



