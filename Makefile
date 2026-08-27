WINDOWS_DIR ?= ./Windows
LINUX_DIR   ?= ./Linux

.PHONY: sync-prod sync-windows sync-linux pull-prod release-windows release-linux \
        release build check verify dry-run clean pipeline-help

sync-prod: sync-windows sync-linux pull-prod

sync-windows:
	git push origin main
	git subtree push --prefix=Windows windows main

sync-linux:
	git push origin main
	git subtree push --prefix=Linux linux main

pull-prod:
	git -C $(WINDOWS_DIR) pull origin main
	git -C $(LINUX_DIR)   pull origin main

release-windows:
	@test -n "$(TAG)" || (echo "Usage: make release-windows TAG=vX.Y.Z" && exit 1)
	git subtree push --prefix=Windows windows main
	git -C $(WINDOWS_DIR) pull origin main
	git -C $(WINDOWS_DIR) fetch --tags
	git tag -a $(TAG) -m "Release $(TAG)" -f
	git push origin $(TAG) -f

release-linux:
	@test -n "$(TAG)" || (echo "Usage: make release-linux TAG=vX.Y.Z" && exit 1)
	git subtree push --prefix=Linux linux main
	git -C $(LINUX_DIR) pull origin main
	git -C $(LINUX_DIR) fetch --tags
	git tag -a $(TAG) -m "Release $(TAG)" -f
	git push origin $(TAG) -f

# ── Local build/release pipeline (see scripts/pipeline.py + RELEASE_PROCESS.md) ──

PYTHON ?= python

## Full pipeline: build → test → security → package → verify → publish (draft)
release:
	@test -n "$(TAG)" || (echo "Usage: make release TAG=vX.Y.Z" && exit 1)
	$(PYTHON) scripts/pipeline.py $(TAG)

## Build + test + security only — run before every commit
check:
	@test -n "$(TAG)" || (echo "Usage: make check TAG=vX.Y.Z" && exit 1)
	$(PYTHON) scripts/pipeline.py $(TAG) --skip package,verify,publish

## Build only
build:
	@test -n "$(TAG)" || (echo "Usage: make build TAG=vX.Y.Z" && exit 1)
	$(PYTHON) scripts/pipeline.py $(TAG) --skip test,security,package,verify,publish

## Re-verify + publish an existing dist/$(TAG)/
verify:
	@test -n "$(TAG)" || (echo "Usage: make verify TAG=vX.Y.Z" && exit 1)
	$(PYTHON) scripts/pipeline.py $(TAG) --from verify

## Dry run — preview all stages without touching anything
dry-run:
	@test -n "$(TAG)" || (echo "Usage: make dry-run TAG=vX.Y.Z" && exit 1)
	$(PYTHON) scripts/pipeline.py $(TAG) --dry-run

## Remove dist/$(TAG)/ output
clean:
	@test -n "$(TAG)" || (echo "Usage: make clean TAG=vX.Y.Z" && exit 1)
	$(PYTHON) -c "import shutil,sys; p='dist/'+sys.argv[1]; shutil.rmtree(p); print('Removed',p)" $(TAG)

pipeline-help:
	@echo ""
	@echo "  VeloceNetwork pipeline targets (require TAG=vX.Y.Z):"
	@echo "    make check   TAG=v4.8.0   build + test + security"
	@echo "    make release TAG=v4.8.0   full pipeline + publish draft"
	@echo "    make build   TAG=v4.8.0   build only"
	@echo "    make verify  TAG=v4.8.0   verify + publish existing dist/"
	@echo "    make dry-run TAG=v4.8.0   preview without changes"
	@echo "    make clean   TAG=v4.8.0   delete dist/vX.Y.Z/"
	@echo ""
	@echo "  See RELEASE_PROCESS.md for the full standardized process."
	@echo ""
