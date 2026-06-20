# env -i PATH=$HOME/.cargo/bin:/usr/bin:/bin:/usr/local/bin        HOME=$HOME        USER=$USER        CC=/usr/bin/clang        CXX=/usr/bin/clang++        CLANG_PATH=/usr/bin/clang        cargo build --release

# env -i PATH=$HOME/.cargo/bin:/usr/bin:/bin:/usr/local/bin        HOME=$HOME        USER=$USER        CC=/usr/bin/clang        CXX=/usr/bin/clang++        CLANG_PATH=/usr/bin/clang        cargo run -- index test/
# env -i PATH=$HOME/.cargo/bin:/usr/bin:/bin:/usr/local/bin        HOME=$HOME        USER=$USER        CC=/usr/bin/clang        CXX=/usr/bin/clang++        CLANG_PATH=/usr/bin/clang        cargo run -- "persoon"

# ln -s lens.service ~/.config/systemd/user/gnome-lens.service
# systemctl --user daemon-reload
# systemctl --user enable --now gnome-lens.service


echo '{"query": "persoon"}' | nc -U ~/.local/state/gnome-lens/gnome_lens.sock