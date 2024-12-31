FROM debian:bookworm-slim AS builder

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update -qq && \
    apt-get install -y --no-install-recommends \
        ca-certificates curl build-essential \
        libssl-dev pkg-config
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:$PATH"

WORKDIR /src
# TODO: reuse crate cache
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /src/target/release/distrans /distrans

WORKDIR /share
COPY --from=builder /src/examples/fly.io/secure-envelopes.png .

VOLUME /state
ENV STATE_DIR=/state

ENV RUST_LOG="veilid_core=error,distrans=debug"
ENV NODE_ADDR=":5150"

EXPOSE 5150/udp
EXPOSE 5150/tcp

ENTRYPOINT ["/distrans", "seed", "/share/secure-envelopes.png"]
