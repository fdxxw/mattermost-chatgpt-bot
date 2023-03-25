#第一阶段用于生成构建文件
FROM messense/rust-musl-cross:x86_64-musl as build

RUN rm -rf ~/.rustup/toolchains/*
RUN rustup update
RUN rustup target add x86_64-unknown-linux-musl

WORKDIR /app

RUN mkdir .cargo
RUN echo "[source.crates-io] \nreplace-with = 'ustc' \n\n[source.ustc] \nregistry = \"git://mirrors.ustc.edu.cn/crates.io-index\"" > .cargo/config.toml
RUN cat .cargo/config.toml

RUN echo "fn main() {}" > dummy.rs
COPY ./Cargo.toml .
RUN sed -i 's#src/main.rs#dummy.rs#' Cargo.toml
RUN cargo build --release

RUN sed -i 's#dummy.rs#src/main.rs#' Cargo.toml

COPY ./src ./src
RUN cargo build --release 

#第二阶段生成最终的Docker镜像
FROM alpine:3.17

#RUN sed -i "s/dl-cdn.alpinelinux.org/mirrors.aliyun.com/g" /etc/apk/repositories
#RUN apk add --no-cache openssl-dev openssl

COPY --from=build /app/target/x86_64-unknown-linux-musl/release/app /opt/app/app

RUN chmod +x /opt/app/app

CMD ["/opt/app/app"]
