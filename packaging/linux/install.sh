#!/bin/sh
# Instala o RustDraw só para o usuário atual (sem sudo): o executável em
# ~/.local/bin, o atalho no menu de aplicativos, o ícone e a associação com
# arquivos .excalidraw e .excalidrawlib (duplo clique abre no RustDraw).
#
#   ./install.sh               instala ou atualiza
#   ./install.sh --uninstall   remove tudo o que a instalação criou
set -eu

here=$(cd "$(dirname "$0")" && pwd)
bin_dir=${XDG_BIN_HOME:-$HOME/.local/bin}
data=${XDG_DATA_HOME:-$HOME/.local/share}
apps=$data/applications
mime=$data/mime
icons=$data/icons/hicolor
sizes="48 64 128 256 512"

refresh() {
    if command -v update-mime-database >/dev/null 2>&1; then
        update-mime-database "$mime" >/dev/null 2>&1 || true
    fi
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$apps" >/dev/null 2>&1 || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q -t "$icons" >/dev/null 2>&1 || true
    fi
}

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$bin_dir/rustdraw" "$apps/rustdraw.desktop" "$mime/packages/rustdraw.xml"
    for s in $sizes; do
        rm -f "$icons/${s}x${s}/apps/rustdraw.png"
    done
    refresh
    echo "RustDraw removido."
    exit 0
fi

mkdir -p "$bin_dir" "$apps" "$mime/packages"
install -m 755 "$here/rustdraw" "$bin_dir/rustdraw"
# o ícone é desenhado pelo próprio executável, em cada tamanho
for s in $sizes; do
    mkdir -p "$icons/${s}x${s}/apps"
    "$bin_dir/rustdraw" icon "$icons/${s}x${s}/apps/rustdraw.png" --size "$s"
done
# caminho completo: o menu nem sempre tem ~/.local/bin no PATH
sed "s|^Exec=rustdraw|Exec=\"$bin_dir/rustdraw\"|" "$here/rustdraw.desktop" > "$apps/rustdraw.desktop"
cp "$here/rustdraw-mime.xml" "$mime/packages/rustdraw.xml"
refresh
if command -v xdg-mime >/dev/null 2>&1; then
    xdg-mime default rustdraw.desktop application/vnd.excalidraw+json application/vnd.excalidrawlib+json || true
fi

echo "RustDraw instalado em $bin_dir/rustdraw."
case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) echo "Para usar no terminal, adicione $bin_dir ao PATH." ;;
esac
