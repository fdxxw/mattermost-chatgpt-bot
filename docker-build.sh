#!/bin/bash

docker build -t fdxxw/`sed -nE 's/^name = "(.*)"/\1/p' Cargo.toml`:`sed -nE 's/^version = "(.*)"/\1/p' Cargo.toml` .