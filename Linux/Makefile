PREFIX  ?= /usr/local
BINDIR   = $(PREFIX)/bin
SYSTEMD_DIR = /etc/systemd/system

.PHONY: build install uninstall deb rpm

build:
	cargo build --release

install: build
	install -Dm755 target/release/veloce-core    $(BINDIR)/veloce-core
	install -Dm755 target/release/veloce-run     $(BINDIR)/veloce-run
	install -Dm755 target/release/veloce-shell   $(BINDIR)/veloce-shell
	install -Dm755 target/release/veloce-launcher $(BINDIR)/veloce-launcher
	$(BINDIR)/veloce-core install
	@echo "VeloceCore installed. Run: systemctl enable --now veloce-core"

uninstall:
	-$(BINDIR)/veloce-core uninstall
	rm -f $(BINDIR)/veloce-core $(BINDIR)/veloce-run
	rm -f $(BINDIR)/veloce-shell $(BINDIR)/veloce-launcher

deb: build
	cargo deb -p veloce-core

rpm: build
	cargo generate-rpm -p crates/veloce-core

clean:
	cargo clean
