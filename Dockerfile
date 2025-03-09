FROM debian:bookworm-slim

RUN apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y ca-certificates curl xz-utils

COPY Cargo.toml .
RUN awk '/^version =/ {print $3}' Cargo.toml | sed 's/"//g' > /stigmerge.version
RUN curl --proto '=https' --tlsv1.2 -LsSf https://github.com/cmars/stigmerge/releases/download/stigmerge-v$(cat /stigmerge.version)/stigmerge-installer.sh | sh
ENV PATH="$HOME/.cargo/bin:$PATH"

VOLUME /share
WORKDIR /share

VOLUME /state
ENV STATE_DIR=/state

ENTRYPOINT ["/root/.cargo/bin/stigmerge"]
CMD ["stigmerge"]
