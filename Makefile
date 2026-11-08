.PHONY: check fmt lint test arch header app install

check: fmt lint test arch

fmt:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

arch:
	cargo arch-lint check
	cargo arch-lint check --rules no-panic-in-lib

header:
	cbindgen --config crates/nimk-ffi/cbindgen.toml --crate nimk-ffi --output target/nimk_ffi.h

app:
	./scripts/build-app.sh

install: app
	./scripts/install.sh
