.PHONY: dev format lint test validate build-docker run-docker-local

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

build-docker:
	docker build --tag rust-catalog-api:local .

run-docker-local: build-docker
	@test -f .env || { echo "error: .env is required" >&2; exit 1; }
	@sed \
		-e '/^DATABASE_URL=/s/@localhost:/@host.docker.internal:/' \
		-e '/^SMTP_URL=/s#//localhost:#//host.docker.internal:#' \
		-e '/^MAIL_FROM="/s/^MAIL_FROM="\(.*\)"$$/MAIL_FROM=\1/' \
		.env | docker run --rm \
			--name rust-catalog-api-local \
			--publish 127.0.0.1:3000:3000 \
			--env-file /dev/stdin \
			--read-only \
			--cap-drop=ALL \
			--security-opt=no-new-privileges \
			rust-catalog-api:local
