# Building the BTX engine for macOS

Two CMake shims and the reason they exist. `.github/workflows/btxd-macos.yml`
uses them, and so does anyone building the engine on a Mac by hand.

## Why the tree's own `depends/` and not Homebrew

The BTX tree pins **Boost 1.81.0** in `depends/packages/boost.mk`. Homebrew ships
whatever is current — 1.92.0 on a GitHub runner in September 2026 — and the two
are not interchangeable: `txmempool.h`'s `boost::multi_index` fails to
instantiate against 1.92, with a wall of static assertions inside Boost itself.
Measured, run 34062662183, after seven minutes of compiling.

So the libraries come from `depends/`, which is what upstream's build system is
for. That also removes every "what did brew upgrade this week" question from the
release path.

## What the shims fix

`depends/` produces static archives, and two of the tree's own find modules
**only look in Homebrew's directories**:

* `cmake/module/FindZeroMQ.cmake` searches `/opt/homebrew/lib` and
  `/usr/local/lib` for `libzmq.a`, and is `FATAL_ERROR` otherwise.
* `cmake/module/FindLibevent.cmake` does the same for `libevent_core.a`,
  `libevent_extra.a` and `libevent_pthreads.a`.

Neither can be redirected with a `-D`. So:

* **`zmq-depends.cmake`** is passed as `-DCMAKE_PROJECT_INCLUDE`. It creates the
  `zeromq` target from the depends archive before the tree's finder runs, and
  prepends `modules/` to `CMAKE_MODULE_PATH`. It guards on `if(TARGET zeromq)
  return()` because a project-include re-runs for **every** nested `project()`
  call — secp256k1 and libbitcoinpqc each trigger it.
* **`modules/FindLibevent.cmake`** overrides the tree's module. It is the tree's
  own macOS branch with the search paths pointed at `depends/`, keeping the same
  target names, the same static linkage, the same
  `evhttp_connection_get_peer` probe and the same `LIBEVENT_LINKAGE` marker.

Both live **outside** the BTX source tree on purpose. Anything written inside it
sets `BUILD_GIT_DIRTY`, and a dirty btxd fails its own production canary with
`build_provenance_mismatch` — a node that runs, syncs and holds peers while
silently refusing to validate.

## What is still Homebrew's job

`libomp`. `src/CMakeLists.txt` takes a hardcoded Homebrew `libomp.a` path
whenever the compiler is AppleClang, and no `-D` redirects it, so
`brew install libomp` is required. (Using a different Clang avoids that branch
but then fails elsewhere — see the workflow header.)

## The SDK symlink

`depends/hosts/darwin.mk` hardcodes an extracted-SDK path
(`Xcode-15.0-15A240d-extracted-SDK-with-libcxx-headers`). Symlink the real SDK
there before running `make -C depends`.

## Doing it by hand

```bash
ln -sfn "$(xcrun --show-sdk-path)" \
  depends/SDKs/Xcode-15.0-15A240d-extracted-SDK-with-libcxx-headers
make -C depends -j"$(sysctl -n hw.ncpu)" HOST=arm64-apple-darwin \
  NO_QT=1 NO_BDB=1 NO_UPNP=1 NO_USDT=1
cmake -B build -G Ninja -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_TOOLCHAIN_FILE=depends/arm64-apple-darwin/toolchain.cmake \
  -DCMAKE_PROJECT_INCLUDE=<this dir>/zmq-depends.cmake \
  -DBUILD_DAEMON=ON -DBUILD_CLI=ON -DBUILD_UTIL=OFF -DBUILD_TX=OFF \
  -DBUILD_WALLET_TOOL=OFF -DBUILD_GUI=OFF -DBUILD_BENCH=OFF \
  -DBUILD_TESTS=OFF -DBUILD_FUZZ_BINARY=OFF \
  -DENABLE_WALLET=ON -DWITH_SQLITE=ON -DWITH_ZMQ=ON \
  -DBTX_ENABLE_METAL=ON -DBTX_MATMUL_METAL_PRECOMPILE_KERNELS=OFF
cmake --build build -j"$(sysctl -n hw.ncpu)"
```

Then confirm, every time, before staging: `BUILD_GIT_DIRTY 0`, `git status
--short` prints nothing, and upstream's own
`scripts/release/verify_release_btxd.py` passes.
