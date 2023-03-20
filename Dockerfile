#第一阶段用于生成构建文件
FROM rust:1.68-alpine3.16 as build

WORKDIR /usr/src/app
COPY ./ .

RUN sed -i "s/dl-cdn.alpinelinux.org/mirrors.aliyun.com/g" /etc/apk/repositories

RUN apk add --no-cache build-base  pkgconfig openssl openssl-dev

#RUN apk add --no-cache cargo

#编译构建文件
RUN mkdir ~/.cargo

RUN echo -e '[source.crates-io] \n replace-with = "tuna" \n [source.tuna] \n registry = "https://mirrors.tuna.tsinghua.edu.cn/git/crates.io-index.git"' > ~/.cargo/config

RUN cat ~/.cargo/config

RUN cargo build --release --target=x86_64-unknown-linux-musl 

#第二阶段生成最终的Docker镜像
FROM alpine:3.16

COPY --from=build /usr/src/app/target/x86_64-unknown-linux-musl/release/mattermost-chatgpt-bot /usr/local/bin/mattermost-chatgpt-bot

ENTRYPOINT ["/usr/local/bin/mattermost-chatgpt-bot"]
