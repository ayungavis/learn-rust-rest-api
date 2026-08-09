.PHONY: dev format lint test

dev:
	cargo run

format:
	cargo fmt

lint:
	cargo clippy --all-targets --all-features --locked -- -D warnings

test:
	cargo test --locked

validate:
	make format && make lint && make test
