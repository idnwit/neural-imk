.PHONY: swift-test check fmt lint test arch header models app install dmg hooks

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

swift-test:
	mkdir -p target/swift-test
	swiftc macos/NeuralIMK/Debouncer.swift macos/tests/debouncer/main.swift -o target/swift-test/debouncer
	target/swift-test/debouncer
	swiftc macos/NeuralIMK/Debouncer.swift macos/NeuralIMK/IdleCommit.swift macos/tests/idlecommit/main.swift -o target/swift-test/idlecommit
	target/swift-test/idlecommit
	swiftc macos/NeuralIMK/StatsText.swift macos/tests/statstext/main.swift -o target/swift-test/statstext
	target/swift-test/statstext
	swiftc macos/NeuralIMK/StatsText.swift macos/NeuralIMK/TuneText.swift macos/tests/tunetext/main.swift -o target/swift-test/tunetext
	target/swift-test/tunetext
	swiftc macos/Shared/*.swift macos/tests/app/main.swift -o target/swift-test/app
	target/swift-test/app

header:
	cbindgen --config crates/nimk-ffi/cbindgen.toml --crate nimk-ffi --output target/nimk_ffi.h

# Pretrains the bundled models on synthetic data (fixed seed) into
# target/models/<kind>/ and copies each checkpoint to
# target/models/bundle/model/<kind>/latest (the layout the app bundles).
# A kind that is already bundled is skipped unless FORCE=1 (GRU takes ~100 s).
MODEL_KINDS := linear_action gru_action
MODELS_DIR := target/models

models:
	@set -e; \
	for kind in $(MODEL_KINDS); do \
	  dest="$(MODELS_DIR)/bundle/model/$$kind/latest"; \
	  if [ -f "$$dest" ] && [ "$(FORCE)" != "1" ]; then \
	    echo "models: $$kind already bundled (FORCE=1 to retrain)"; continue; \
	  fi; \
	  echo "models: pretraining $$kind"; \
	  rm -rf "$(MODELS_DIR)/$$kind"; \
	  cargo run --release -p nimk-synth --bin nimk-synth -- pretrain \
	    --model $$kind --out "$(MODELS_DIR)/$$kind" \
	    --seed 42 --sessions 200 --words 500 --epochs 2; \
	  mkdir -p "$$(dirname "$$dest")"; \
	  cp "$(MODELS_DIR)/$$kind/blobs/model/$$kind/latest" "$$dest"; \
	done

app:
	./scripts/build-app.sh

install: app
	./scripts/install.sh

dmg:
	./scripts/package-dmg.sh

# Git hooks in .githooks/ (commit-msg: Conventional Commits).
hooks:
	git config core.hooksPath .githooks
