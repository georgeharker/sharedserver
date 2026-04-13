.PHONY: test lint

# Locate vusted: prefer luarocks-managed binary, fall back to PATH
VUSTED := $(firstword \
    $(wildcard $(HOME)/.luarocks/bin/vusted) \
    $(shell which vusted 2>/dev/null) \
)

test:
	$(VUSTED) tests/spec/

lint:
	luac -p lua/sharedserver/init.lua && echo "Syntax OK"
