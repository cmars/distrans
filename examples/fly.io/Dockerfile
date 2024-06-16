FROM debian:bookworm-slim

RUN apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y ca-certificates curl xz-utils

COPY Cargo.toml .
RUN awk '/^version =/ {print $3}' Cargo.toml | sed 's/"//g' > /distrans.version
RUN curl --proto '=https' --tlsv1.2 -LsSf https://github.com/cmars/distrans/releases/download/distrans_cli-v$(cat /distrans.version)/distrans_cli-installer.sh | sh
ENV PATH="$HOME/.cargo/bin:$PATH"

WORKDIR /share
COPY examples/fly.io/secure-envelopes.png .

VOLUME /state
ENV STATE_DIR=/state

# TODO: fix cli app to tolerate not being in a tty. Without this bit of magic it seems to hang.
ENTRYPOINT ["/bin/sh"]
CMD ["-c", "script -qefc '/root/.cargo/bin/distrans seed /share/secure-envelopes.png'"]