PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
SYSTEMD_USER_DIR ?= $(PREFIX)/lib/systemd/user

.PHONY: all build install uninstall clean

all: build

build:
	cargo build --release

install: 
	# Install binaries
	install -Dm755 target/release/bluwofi $(DESTDIR)$(BINDIR)/bluwofi
	install -Dm755 target/release/bluetooth-reconnectd $(DESTDIR)$(BINDIR)/bluetooth-reconnectd
	# Install systemd user service system-wide
	install -Dm644 systemd/bluetooth-reconnectd.service $(DESTDIR)$(SYSTEMD_USER_DIR)/bluetooth-reconnectd.service
	@echo ""
	@echo "======================================================================"
	@echo " INSTALLATION COMPLETE"
	@echo "======================================================================"
	@echo "Binaries installed to:   $(BINDIR)"
	@echo "Systemd service to:      $(SYSTEMD_USER_DIR)"
	@echo ""
	@echo "--> SWAY CONFIGURATION"
	@echo "Add the following to ~/.config/sway/config:"
	@echo "  bindsym \$$mod+b exec $(BINDIR)/bluwofi"
	@echo ""
	@echo "--> WAYBAR CONFIGURATION"
	@echo "Add this module to ~/.config/waybar/config (or config.jsonc):"
	@echo "  \"custom/bluetooth\": {"
	@echo "      \"exec\": \"$(BINDIR)/bluwofi --status\","
	@echo "      \"return-type\": \"json\","
	@echo "      \"interval\": 5,"
	@echo "      \"on-click\": \"$(BINDIR)/bluwofi\","
	@echo "      \"format\": \"{}\""
	@echo "  }"
	@echo ""
	@echo "--> AUTO-RECONNECT DAEMON"
	@echo "To enable the background reconnection service for your user, run:"
	@echo "  systemctl --user daemon-reload"
	@echo "  systemctl --user enable --now bluetooth-reconnectd"
	@echo "======================================================================"

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/bluwofi
	rm -f $(DESTDIR)$(BINDIR)/bluetooth-reconnectd
	rm -f $(DESTDIR)$(SYSTEMD_USER_DIR)/bluetooth-reconnectd.service
	@echo "Uninstalled successfully."

clean:
	cargo clean
