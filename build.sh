#!/bin/sh
# 建置引擎 WASM 並打包網頁（輸出 web/dist/index.html 單一檔案）
set -e
cd "$(dirname "$0")"
(cd engine && wasm-pack build --target web --release --out-dir ../web/src/pkg -- --features wasm)
(cd web && npm install --silent && npm run build)
(cd web && python3 to_artifact.py)
