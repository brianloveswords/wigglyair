BIN = wigglyair

NAS_TRIPLE = x86_64-unknown-linux-gnu
RELEASE_TARGET = target/${NAS_TRIPLE}/release/${BIN}

ifndef NAS_PATH
$(error NAS_PATH is not set)
endif

install-to-nas : ${RELEASE_TARGET}
	scp -O ${^} ${NAS_PATH}

${RELEASE_TARGET} : src/main.rs
	cross build --release --target ${NAS_TARGET}

debug :
	cargo build

release :
	cargo build --release

run :
	cargo run

