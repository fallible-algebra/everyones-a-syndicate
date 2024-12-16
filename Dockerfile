FROM rust:1.80-bookworm as dependencies
RUN apt update
RUN apt install -y openssl libjemalloc2 zlib1g

FROM dependencies as build
COPY . /usr/bin/syndicate
WORKDIR /usr/bin/syndicate
RUN cargo build --bin everyones-a-syndicate

FROM build as run
WORKDIR /usr/bin/syndicate
EXPOSE 3000
CMD cargo run --bin everyones-a-syndicate