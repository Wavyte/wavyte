1. We are working on a rust-based project

2. We are working on a cloud VPS with rust tooling installed, and are on a git branch

3. No markdown other than README.md is to be git tracked

4. Before making any git commit, the standard rust quality passes: test, fmt, and clippy, must be complied with.

5. During dev, all builds should be dev. For speed you can quickly do check instead of full build while iterating

6. We must strictly enforce no usage of unsafe code.

7. Rust builds and test take time, so that into account while running the code -> test -> iterate loop

8. Write commit messages as "v0.x.0 - {brief description of work}" where x is the current MINOR version being worked upon. Currently x=2 on this branch.

9. Always strictly ground in latest docs from web search. Do not make assumptions

10. You are free to update this doc for your own alignment.

11. Ensure that all path handling logic in the codebase is OS agnostic and strictly relative paths.

12. (New Instruction) Added sample assets for testing in assets/ directory