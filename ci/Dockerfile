# Cross-build image for Padd, run on the Mac mini (Colima, arm64 Linux VM).
# Debian bookworm => glibc 2.36, so binaries run on Ubuntu 22.04+ / Debian 12+.
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl gcc libc6-dev \
        gcc-x86-64-linux-gnu libc6-dev-amd64-cross \
        gcc-mingw-w64-x86-64 \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable \
    && rustup component add rustfmt clippy \
    && rustup target add x86_64-unknown-linux-gnu x86_64-pc-windows-gnu
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
    CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
    CARGO_TERM_COLOR=never
WORKDIR /src
