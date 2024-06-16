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

# TODO: fix cli app to tolerate not being in a tty. Without this bit of magic it seems to hang.
ENTRYPOINT ["/bin/sh"]
CMD ["-c", "script -qefc '/distrans seed /share/secure-envelopes.png'"]