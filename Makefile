.DEFAULT_GOAL := help

NIX_PROFILE_OPTIONS ?=

.PHONY: help install install-bas install-bas-service install-basg

# Installers update the same Nix profile, so keep them sequential even with -j.
.NOTPARALLEL:

help:
	@printf '%s\n' \
		'make install              Install bas, bast and basg into your Nix profile' \
		'make install-bas          Install bas and bast' \
		'make install-bas-service  Install bas and enable/restart its systemd user service' \
		'make install-basg         Install basg with its desktop entry and graphics libraries' \
		'' \
		'Pass Nix options with NIX_PROFILE_OPTIONS, e.g. NIX_PROFILE_OPTIONS="--profile /path/to/profile -L"'

install: install-bas install-basg

install-bas:
	./scripts/install-bas $(NIX_PROFILE_OPTIONS)

install-bas-service:
	./scripts/install-bas --service $(NIX_PROFILE_OPTIONS)

install-basg:
	./scripts/install-basg $(NIX_PROFILE_OPTIONS)
