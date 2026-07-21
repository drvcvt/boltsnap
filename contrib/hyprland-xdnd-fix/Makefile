CXX ?= g++
CXXFLAGS ?= -O2
CXXFLAGS += -shared -fPIC -std=c++23

ifeq ($(CXX),g++)
EXTRA_FLAGS = --no-gnu-unique
endif

.PHONY: all clean test

all:
	$(CXX) $(CXXFLAGS) $(LDFLAGS) $(EXTRA_FLAGS) main.cpp -o boltsnap-xdnd-fix.so \
		`pkg-config --cflags pixman-1 libdrm hyprland`

clean:
	rm -f boltsnap-xdnd-fix.so

test:
	./tests/check.sh
