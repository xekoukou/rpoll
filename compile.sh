#!/bin/sh

cd meta
ribosome.js crate.js.dna
cd ../
cargo build
