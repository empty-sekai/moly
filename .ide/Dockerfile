FROM cnbcool/default-dev-env:latest

USER root

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       build-essential clang lld cmake ninja-build pkg-config libasound2-dev \
       libudev-dev libx11-dev libxkbcommon-dev libwayland-dev \
       ca-certificates curl git \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain stable \
    && /root/.cargo/bin/rustup target add wasm32-unknown-unknown \
    && /root/.cargo/bin/cargo install wasm-bindgen-cli --version 0.2.127 --locked

ENV PATH="/root/.cargo/bin:${PATH}"
