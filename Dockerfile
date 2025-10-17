FROM alpine:latest

RUN apk add --update --no-cache --repository https://dl-3.alpinelinux.org/alpine/latest-stable/community --repository https://dl-3.alpinelinux.org/alpine/latest-stable/main rust cargo openssl-dev

WORKDIR /opt/uptime

COPY ./Cargo.toml ./Cargo.toml

ADD . ./

RUN cargo build --release

FROM alpine:latest

RUN apk add --update --no-cache --repository https://dl-3.alpinelinux.org/alpine/latest-stable/community --repository https://dl-3.alpinelinux.org/alpine/latest-stable/main libgcc memcached

WORKDIR /opt/uptime

COPY --from=0 /opt/uptime/target/release/uptime-check ./

EXPOSE 8080

ENV RUST_BACKTRACE=1

ENV RUST_LOG=info,reqwest=warn,hyper_util::client::legacy::connect::http=warn,hyper_util::client::legacy::pool=warn,hyper_util::client::legacy::connect::dns=warn

RUN echo '#!/bin/sh' > ./start.sh && \
    echo 'memcached -u root -d' >> ./start.sh && \
    echo 'exec ./uptime-check' >> ./start.sh && \
    chmod +x ./start.sh

CMD ["./start.sh"]