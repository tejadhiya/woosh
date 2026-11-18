# Woosh -- developer storage reclaimer for Apple Silicon.
#
# Every build + verification step runs in Docker (`DC` targets).
# The ONLY host-side targets are `dev`, `app`, `dmg`, `release` and `verify-macos`:
# bundling a macOS .app requires WKWebView + codesign, which no Linux container has.
# They use the host's pre-existing rust 1.94 / pnpm -- nothing is ever installed.

DC := docker compose -f docker/docker-compose.yml
UI := app
TARGET := aarch64-apple-darwin

.DEFAULT_GOAL := help
.PHONY: help images fmt clippy test bench check-workspace check ci ui shell \
        fixture clean dev app dmg release verify-macos doctor

help: ## Show this help
	@grep -hE '^[a-z-]+:.*?## ' $(MAKEFILE_LIST) \
	  | awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-16s\033[0m %s\n",$$1,$$2}'

# ---------------------------------------------------------------- containerized
images: ## Build the Docker build/verify images
	$(DC) build

fmt: ## Check formatting (Docker)
	$(DC) run --rm fmt

clippy: ## Lint the engine, warnings are errors (Docker)
	$(DC) run --rm clippy

test: ## Run the engine test suite (Docker)
	$(DC) run --rm test

bench: fixture ## Run the sweep benchmark (Docker)
	$(DC) run --rm bench

check-workspace: ## Typecheck the whole workspace incl. Tauri shell (Docker)
	$(DC) run --rm check-workspace

ui: ## Typecheck + bundle the frontend (Docker)
	$(DC) run --rm ui

check: fmt clippy test ## fmt + clippy + test
ci: images fixture check check-workspace ui bench ## Everything a CI run would do

shell: ## Interactive shell in the full build image (Docker)
	$(DC) run --rm shell

# ---------------------------------------------------------------------- fixtures
fixture: ## Generate the known-answer test fixture tree
	@bash scripts/make-fixture.sh .fixture

# --------------------------------------------------------------------- host-only
doctor: ## Report host toolchain + signing readiness
	@bash scripts/doctor.sh

dev: ## Run the app in dev mode (HOST -- needs WKWebView)
	cd $(UI) && pnpm install && pnpm exec tauri dev

# APPLE_SIGNING_IDENTITY must be exported *into the build*, not applied after it.
# `tauri build` produces the .app and the .dmg in one pass, so signing afterwards
# leaves the DMG holding a copy of the unsigned app -- which macOS reports to the
# recipient as "Woosh is damaged and can't be opened". Defaults to "-" (ad-hoc).
app: ui ## Bundle the macOS .app, signed before the DMG is made (HOST -- see plan 0.1)
	cd $(UI) && pnpm install && \
	  APPLE_SIGNING_IDENTITY="$${APPLE_SIGNING_IDENTITY:--}" \
	  pnpm exec tauri build --target $(TARGET)

dmg: app ## Build the shareable DMG (HOST)
	@bash scripts/make-dmg.sh

release: ## Signed + notarized DMG (HOST; needs APPLE_* env, see plan 8.1)
	@bash scripts/release.sh

verify-macos: ## Differential: macOS getattrlistbulk vs portable walker (HOST)
	cargo test -p woosh-engine --all-targets -- --nocapture

clean: ## Remove build output and Docker volumes
	$(DC) down -v --remove-orphans || true
	rm -rf target $(UI)/dist $(UI)/node_modules .fixture
