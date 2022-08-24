FROM rust:1.62.0-slim-bullseye
RUN apt update && apt install -y \
    git \
    pkg-config \
    libssl-dev
