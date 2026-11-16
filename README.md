# Remote Files — a SimpleEdit plugin

Files of another machine, over SSH, in [SimpleEdit](https://github.com/simple-edit-app/simpleedit). The system's `ssh` does the work: keys, the agent and `~/.ssh/config` apply. Give `user@host` and a folder in its panel, pick a file: a copy is kept here and opened like any file, and sent back at each save (unless the file changed on the machine since). `user@host:/path/file` opens that file at once.

## Build

```sh
cargo test
cargo build --release && mkdir -p bin && cp target/release/remote bin/
```

## License

MIT
