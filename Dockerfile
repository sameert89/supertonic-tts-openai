FROM rust:1.92 AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y cmake pkg-config libssl-dev ffmpeg tini && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./

RUN mkdir src
RUN mkdir assets

COPY src ./src
COPY assets ./assets

RUN cargo build --release

EXPOSE 8080

RUN ls -a ./target/release/

ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["./target/release/server"]
