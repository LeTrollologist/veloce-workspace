## VeloceNetwork dev workspace — production sync targets
##
## Production repos:
##   windows → https://github.com/LeTrollologist/VeloceNetwork-Windows
##   linux   → https://github.com/LeTrollologist/VeloceNetwork-Linux
##
## Local production clones:
##   /c/Users/Owner/Windows   (VeloceNetwork-Windows)
##   /c/Users/Owner/Linux     (VeloceNetwork-Linux)

WINDOWS_DIR := /c/Users/Owner/Windows
LINUX_DIR   := /c/Users/Owner/Linux

.PHONY: sync-prod sync-windows sync-linux pull-prod \
        release-windows release-linux help

## ── Sync ──────────────────────────────────────────────────────────────────────

## Push latest main to both production repos, then pull into local clones.
sync-prod: sync-windows sync-linux pull-prod

## Push main to origin (veloce-workspace) and VeloceNetwork-Windows.
sync-windows:
	git push origin main
	git push windows main

## Push main to origin (veloce-workspace) and VeloceNetwork-Linux.
sync-linux:
	git push origin main
	git push linux main

## Pull latest from their remotes into both local production clones.
pull-prod:
	git -C $(WINDOWS_DIR) pull origin main
	git -C $(LINUX_DIR)   pull origin main

## ── Release ───────────────────────────────────────────────────────────────────
## Usage:
##   make release-windows TAG=v2.1.0
##   make release-linux   TAG=v1.1.0

## Tag, push to origin + Windows remote, refresh Windows clone.
release-windows:
	@test -n "$(TAG)" || (echo "Usage: make release-windows TAG=vX.Y.Z" && exit 1)
	git tag $(TAG)
	git push origin main $(TAG)
	git push windows main $(TAG)
	git -C $(WINDOWS_DIR) pull origin main
	git -C $(WINDOWS_DIR) fetch --tags

## Tag (prefixed linux-), push to origin + Linux remote, refresh Linux clone.
## Linux tags are prefixed 'linux-' to avoid collision with Windows tags.
release-linux:
	@test -n "$(TAG)" || (echo "Usage: make release-linux TAG=vX.Y.Z" && exit 1)
	git tag linux-$(TAG)
	git push origin main linux-$(TAG)
	git push linux main linux-$(TAG)
	git -C $(LINUX_DIR) pull origin main
	git -C $(LINUX_DIR) fetch --tags

## ── Help ──────────────────────────────────────────────────────────────────────

help:
	@echo ""
	@echo "  make sync-prod               Push main to both prod repos + pull local clones"
	@echo "  make sync-windows            Push main to VeloceNetwork-Windows only"
	@echo "  make sync-linux              Push main to VeloceNetwork-Linux only"
	@echo "  make pull-prod               Pull latest into Windows/ and Linux/ clones"
	@echo "  make release-windows TAG=vX  Tag + push Windows release"
	@echo "  make release-linux   TAG=vX  Tag (linux-vX) + push Linux release"
	@echo ""
