/* Non-loaded compiler metadata, like a Rust dylib's .rustc export. */
.section .rustc,"",@progbits
.globl rust_metadata_fixture
.type rust_metadata_fixture,@object
rust_metadata_fixture:
.zero 8
.size rust_metadata_fixture, .-rust_metadata_fixture
.section .note.GNU-stack,"",@progbits
