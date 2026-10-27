use runix::prelude::*;

fn main() -> Result<()> {
    let mut system = System::init(&[&Pacman, &Cargo])?;

    system.pkgs(
        Pacman,
        &[
            "base",
            "glibc",
            "pacman",
            "sudo",
            "systemd",
            "fish",
            "cachyos/zen-browser-bin",
        ],
    );
    system.pkgs(Cargo, &["ripgrep"]);
    system.ignore(Pacman, &["linux-cachyos", "linux-cachyos-headers"]);

    system.link("examples/dotfiles/hello", "~/.config/runix-example");
    system
        .link_template(
            "examples/dotfiles/hello/config.toml",
            "~/.config/runix-example/config.toml",
        )
        .replace("{{HOSTNAME}}", system.hostname());

    if system.flag("docker") {
        system.pkgs(Pacman, &["docker", "docker-compose"]);
    }

    system.apply()
}
