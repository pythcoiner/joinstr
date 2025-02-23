binding:
    ./contrib/bindings.sh

clean:
    rm -fRd target
    rm -fRd rust/joinstr/include/c
    rm -fRd rust/joinstr/include/cpp
    rm -fRd dart/lib/joinstr.dart
    rm -fRd dart/android
    rm -fRd dart/ios
    rm -fRd dart/.dart_tool


test:
    cargo clippy --all-features --all-targets -- -D warnings
    cargo test -- --nocapture
