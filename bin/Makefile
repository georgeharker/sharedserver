# Makefile for sharedserver.nvim bin tools

CC = gcc
CFLAGS = -O2 -Wall -Wextra
PREFIX ?= $(HOME)/.local

.PHONY: all clean install install-wrapper install-watcher

all: sharedserver-watcher

sharedserver-watcher: sharedserver-watcher.c
	$(CC) $(CFLAGS) -o $@ $<

clean:
	rm -f sharedserver-watcher

install: install-wrapper install-watcher

install-wrapper:
	install -m 755 sharedserver-wrapper $(PREFIX)/bin/

install-watcher: sharedserver-watcher
	install -m 755 sharedserver-watcher $(PREFIX)/bin/

# For development testing
test: all
	@echo "Running basic tests..."
	@./test-wrapper.sh
