WINDOWS_DIR ?= ./Windows
LINUX_DIR   ?= ./Linux

.PHONY: sync-prod sync-windows sync-linux pull-prod release-windows release-linux

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
