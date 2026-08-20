DOCKER ?= docker
DOCKERFILE := docker/Dockerfile
LINUX_IMAGE := ide-linux-builder
RUN_LINUX = $(DOCKER) run --rm -v "$(CURDIR)":/workspace -w /workspace $(LINUX_IMAGE)

.PHONY: help all test lint build build-linux build-windows linux-image shell clean

.DEFAULT_GOAL := help

help: ## Show this help
	@grep -hE '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'

all: test build ## Run tests, then build all targets

linux-image: ## Build the linux-builder Docker image
	$(DOCKER) build --target linux-builder -t $(LINUX_IMAGE) -f $(DOCKERFILE) .

test: linux-image ## Run cargo test --workspace in Docker
	$(RUN_LINUX) cargo test --workspace

lint: linux-image ## Run clippy + rustfmt check in Docker
	$(RUN_LINUX) cargo clippy --workspace --all-targets -- -D warnings
	$(RUN_LINUX) cargo fmt --all -- --check

build: build-linux build-windows ## Build Linux and Windows artifacts

build-linux: ## Export dist/ide-linux-x86_64/ (binary + bundled Qt runtime)
	$(DOCKER) buildx build --target linux-artifact -f $(DOCKERFILE) \
		--output type=local,dest=dist/ .

# Requires the out-of-band mxe-spike-snapshot:2 base image.
build-windows: ## Export dist/windows/ (needs mxe-spike-snapshot:2)
	$(DOCKER) buildx build --target windows-artifact -f $(DOCKERFILE) \
		--output type=local,dest=dist/ .

shell: linux-image ## Open a bash shell in the builder image
	$(DOCKER) run --rm -it -v "$(CURDIR)":/workspace -w /workspace $(LINUX_IMAGE) bash

clean: ## cargo clean + remove dist/
	$(RUN_LINUX) cargo clean
	rm -rf dist
