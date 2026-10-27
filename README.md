# Runix

Runix is a Rust framework for declarative OS configuration.

## Setup
1. Run `cargo new` somewhere on your computer, I recommend doing this in `~/dotfiles`, but that's up to you.
2. Run `cargo add runix --git "<todo>"`. This will add runix as a dependency to your computer
3. Put the following code in your `src/main.rs` file:
```rust
use runix::prelude::*;

fn main() -> Result<()> {
    // Create the system with Yay and Cargo as providers
    let mut system = System::init(&[&Yay, &Cargo])?;

    // Apply the system changes
    system.apply()
}
```
4. Run `cargo run -- --import`. This will retrieve all the packages you should add to your file for the currently selected providers (which are now managed by runix)

## Setting up aliases so `runix` works

Each alias runs your config from any directory. Replace `~/dotfiles` with the path to your system.

- Fish

    ```fish
    alias runix='cargo run -q --manifest-path ~/dotfiles/Cargo.toml --'
    ```

- Bash/ZSH

    ```bash
    alias runix='cargo run -q --manifest-path ~/dotfiles/Cargo.toml --'
    ```

- Nushell

    ```nu
    def --wrapped runix [...args] {
        cargo run -q --manifest-path ($nu.home-path | path join dotfiles Cargo.toml) -- ...$args
    }
    ```
