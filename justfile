binding:
    ./contrib/bindings.sh

clean:
    rm -fRd target
    rm -fRd rust/joinstr/include/*
    rm -fRd rust/joinstr_wallet/include/*
    rm -fRd rust/qt_joinstr/include/*
    rm -fRd dart/lib/joinstr.dart
    rm -fRd dart/android
    rm -fRd dart/ios
    rm -fRd dart/.dart_tool

lint:
    set -e
    cargo fmt -- --check
    cargo clippy --all-features --all-targets -- -D warnings

test:
    just lint
    cargo test -- --nocapture

wallet:
    cbindgen --lang c  --crate joinstr_wallet  -o rust/joinstr_wallet/include/c/joinstr.h 
    cbindgen --crate joinstr_wallet  -o rust/joinstr_wallet/include/cpp/joinstr.h 
    cargo build -p joinstr_wallet --release
    cp target/release/libjoinstr_wallet.a rust/joinstr_wallet/include/libjoinstr_wallet.a
    cp target/release/libjoinstr_wallet.d rust/joinstr_wallet/include/libjoinstr_wallet.d
    cp target/release/libjoinstr_wallet.so rust/joinstr_wallet/include/libjoinstr_wallet.so

qt:
    just clean
    ./rust/qt_joinstr/build.sh
