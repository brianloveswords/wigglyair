build:
    cargo +nightly build -Z build-std=std,panic_abort --target aarch64-apple-darwin --release
