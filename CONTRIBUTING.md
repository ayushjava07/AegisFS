# Contributing to AegisFS

Thank you for considering contributing to AegisFS. Please follow these guidelines.

## Getting Started

1. Ensure you have Rust 1.82.0+ installed.
2. Clone the repository.
3. Run `cargo build` to verify your build works.

## Code Style

- Run `cargo fmt` before committing.
- Ensure `cargo clippy --all-targets --all-features` produces no warnings.
- Follow the existing patterns in the codebase.

## Testing

- All new code should include tests.
- Run `cargo test --all-features` to run all tests.
- Property-based tests use `proptest`. Add new property tests in `tests/property_tests.rs`.
- Benchmarks live in `benches/` and use Criterion.

## Pull Request Process

1. Create a feature branch from `main`.
2. Make your changes.
3. Run the full test suite.
4. Ensure CI passes.
5. Submit a PR with a clear description of the changes.

## License

Contributions are licensed under the MIT OR Apache-2.0 license.
