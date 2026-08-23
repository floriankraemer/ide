DOCKER ?= docker
DOCKERFILE := docker/Dockerfile
LINUX_IMAGE := ide-linux-builder
# Named volumes, not bind mounts: the crate registry and the ccache object
# store must outlive `--rm`, and neither belongs in the source tree. Without
# them every container start re-downloads the registry and recompiles every
# C++ translation unit from scratch.
DOCKER_MOUNTS = -v "$(CURDIR)":/workspace -w /workspace \
	-v ide-cargo-registry:/usr/local/cargo/registry \
	-v ide-ccache:/ccache
RUN_LINUX = $(DOCKER) run --rm $(DOCKER_MOUNTS) $(LINUX_IMAGE)

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

lint: linux-image ## Run clippy + rustfmt + file-size checks in Docker
	$(RUN_LINUX) cargo clippy --workspace --all-targets -- -D warnings
	$(RUN_LINUX) cargo fmt --all -- --check
	$(RUN_LINUX) scripts/check-file-size.sh

build: build-linux build-windows ## Build Linux and Windows artifacts

build-linux: ## Export dist/ide-linux-x86_64/ (binary + bundled Qt runtime)
	$(DOCKER) buildx build --target linux-artifact -f $(DOCKERFILE) \
		--output type=local,dest=dist/ .

# First run builds the MXE mingw-w64 + Qt6 cross toolchain (mxe-base stage)
# from source: several hours. Cached as a layer afterwards.
build-windows: ## Export dist/windows/ (first run builds the MXE toolchain, hours)
	$(DOCKER) buildx build --target windows-artifact -f $(DOCKERFILE) \
		--output type=local,dest=dist/ .

shell: linux-image ## Open a bash shell in the builder image
	$(DOCKER) run --rm -it $(DOCKER_MOUNTS) $(LINUX_IMAGE) bash

clean: ## cargo clean + remove dist/
	$(RUN_LINUX) cargo clean
	rm -rf dist
