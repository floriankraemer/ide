DOCKER ?= docker
DOCKERFILE := docker/Dockerfile
LINUX_IMAGE := ide-linux-builder
LSP_IMAGE := ide-lsp-conformance
# Named volumes, not bind mounts: the crate registry and the ccache object
# store must outlive `--rm`, and neither belongs in the source tree. Without
# them every container start re-downloads the registry and recompiles every
# C++ translation unit from scratch.
DOCKER_MOUNTS = -v "$(CURDIR)":/workspace -w /workspace \
	-v ide-cargo-registry:/usr/local/cargo/registry \
	-v ide-ccache:/ccache
RUN_LINUX = $(DOCKER) run --rm $(DOCKER_MOUNTS) $(LINUX_IMAGE)

.PHONY: help all test lint e2e e2e-repeat build build-linux build-windows linux-image shell clean \
	lsp-image lsp-conformance

.DEFAULT_GOAL := help

help: ## Show this help
	@grep -hE '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'

all: test build ## Run tests, then build all targets

linux-image: ## Build the linux-builder Docker image
	$(DOCKER) build --target linux-builder -t $(LINUX_IMAGE) -f $(DOCKERFILE) .

test: linux-image ## Run cargo test --workspace in Docker
	$(RUN_LINUX) cargo test --workspace

# The conformance suite runs against a REAL language server, so it is opt-in:
# it needs its own image, takes minutes, and can go red because upstream
# changed rather than because we did. Nightly and on demand, never per-PR.
lsp-image: ## Build the lsp-conformance image (linux-builder + rust-analyzer)
	$(DOCKER) build --target lsp-conformance -t $(LSP_IMAGE) -f $(DOCKERFILE) .

lsp-conformance: lsp-image ## Check the LSP client against a real rust-analyzer
	$(DOCKER) run --rm $(DOCKER_MOUNTS) $(LSP_IMAGE) \
		cargo test -p lsp-core --test real_server_conformance -- --ignored --nocapture

lint: linux-image ## Run clippy + rustfmt + file-size checks in Docker
	$(RUN_LINUX) cargo clippy --workspace --all-targets -- -D warnings
	$(RUN_LINUX) cargo fmt --all -- --check
	$(RUN_LINUX) scripts/check-file-size.sh

# One X server with N app instances makes xdotool's window targeting
# ambiguous, and ambiguous input is the first source of E2E flake — hence
# --test-threads=1. xvfb, xauth, x11-apps, imagemagick and xdotool are already
# in linux-builder, so no image change is needed.
E2E_XVFB = xvfb-run -a --server-args="-screen 0 1600x1200x24"

e2e: linux-image ## Run the E2E flows under Xvfb (ignored by `make test`)
	$(RUN_LINUX) sh -c 'cargo build -p app && $(E2E_XVFB) \
		cargo test -p app --test e2e -- --ignored --test-threads=1 --nocapture'

# Burn-in: `make e2e-repeat TEST=e2e_open_project_edit_save N=20`. A flake is
# a P1 bug in the product or the harness, so this exists to find one before
# it is discovered by somebody re-running CI.
N ?= 20
e2e-repeat: linux-image ## Repeat one E2E flow N times: make e2e-repeat TEST=<name> N=20
	@test -n "$(TEST)" || { echo "usage: make e2e-repeat TEST=<name> [N=20]"; exit 2; }
	$(RUN_LINUX) sh -c 'cargo build -p app && for i in $$(seq 1 $(N)); do \
		echo "--- run $$i/$(N) ---"; \
		$(E2E_XVFB) cargo test -p app --test e2e -- --ignored --exact \
			--test-threads=1 --nocapture $(TEST) || exit 1; \
	done'

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
