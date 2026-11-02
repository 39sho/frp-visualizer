# frp-visualizer

[![CI](https://github.com/39sho/frp-visualizer/actions/workflows/ci.yml/badge.svg)](https://github.com/39sho/frp-visualizer/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

FRP（Functional Reactive Programming）の **Stream / Cell ネットワークを JSON で定義し、離散トランザクションでシミュレーションして SVG に出力する CLI ツール**です。ネットワーク図とタイミング図を同時に生成します。

![Calibration](docs/examples/calibration.svg)

## Features

- `map` / `filter` / `merge` / `snapshot` / `hold` / `lift` などの FRP プリミティブ
- Stream と Cell の型付きポート、離散トランザクションによるシミュレーション
- `network.svg` / `timing.svg` / `diagram.svg` / `diagram.json` の出力
- **orthogonal 配線**（デフォルト）と `--route hybrid` の斜め配線
- モジュールを避けるルーティング、平行線の分離、交差部のホップ
- Cell を帯で示すタイミング図、トランザクション開始／終了ティック
- **Component 雲**（入出力ポート・多出力・内部ネットワークの展開表示・内部状態を含むシミュレーション）
- Node / Edge / Transaction に紐づく**注釈吹き出し**
- `--focus-transaction N` による実行フォーカスパネル
- 手動レイアウトと `--auto-layout` の両対応
- ストリームは実線、セルは青の破線（モノクロでも判別可能）

## Install

Rust ツールチェーンがある場合:

```sh
cargo install --git https://github.com/39sho/frp-visualizer --locked
```

リポジトリ内で開発する場合:

```sh
cargo install --path . --locked
```

`v*` タグを push すると、GitHub Actions が Linux / macOS / Windows 向けのビルド済みバイナリを Releases に添付します。Unix 系のバイナリはダウンロード後に `chmod +x` してください。

## Quick start

```sh
# 組み込み example の一覧
frp-visualizer list

# JSON として書き出す
frp-visualizer example accumulator accumulator.json

# network.svg / timing.svg / diagram.svg / diagram.json を出力
frp-visualizer render accumulator.json out

# 組み込み example を直接出力
frp-visualizer render-example calibration out --auto-layout

# トランザクション T2 をフォーカス
frp-visualizer render-example calibration out --focus-transaction 2

# Component の内部ネットワークも出力
frp-visualizer render-example component out --expand accumulate
```

出力先を省略すると `frp_export/` に書き出します。

## CLI

```text
frp-visualizer list
frp-visualizer example <name> [out.json]
frp-visualizer render <diagram.json> [out_dir] [options]
frp-visualizer render-example <name> [out_dir] [options]
```

| Option | Description |
| --- | --- |
| `--auto-layout` | ノード位置を自動計算 |
| `--route orthogonal` | 水平・垂直配線（デフォルト） |
| `--route hybrid` | 斜め配線を許可 |
| `--focus-transaction N` | N 番目（1 始まり）のトランザクションを実行フォーカス |
| `--expand <component>` | 指定 Component の内部ネットワークを `expanded_<name>.svg` / `.json` に出力 |

## Examples

| Name | Description |
| --- | --- |
| `accumulator` | `hold` / `snapshot` / `merge` による累積器 |
| `map_filter` | Stream の map と filter |
| `snapshot` | Cell のサンプリング |
| `merge` | 2 本の Stream のマージ |
| `lift` | Cell 同士の合成 |
| `calibration` | クリア・スケーリング付き累積器 |
| `component` | Component 雲と多出力、内部状態 |
| `annotation` | 注釈吹き出し |

### Components

`Component` は入出力ポートを持つ雲として表示され、内部に `Document` を持てます。シミュレーション時は階層をフラット化して内部の `hold` / `snapshot` まで実行します。

![Component](docs/examples/component.svg)

`--expand` で内部ネットワークを単体表示できます。

![Expanded component](docs/examples/expanded-accumulate.svg)

### Annotations

注釈は Node / Edge / Transaction を対象に、緑の吹き出しと楔形ポインタで描画されます。

![Annotations](docs/examples/annotations.svg)

### Execution focus

![Execution](docs/examples/execution.svg)

## JSON format

`frp-visualizer example accumulator` で完全な例を確認できます。主なフィールド:

- `nodes`: `id` / `kind` / `name` / `pos` / `note` / `label`
- `edges`: `from` / `from_port` / `to` / `to_port` / `label`
- `components`: `name` / `inputs` / `outputs` / `body` / `doc`
- `annotations`: `text` / `target` / `position`
- `time_end`: タイミング図の終端時刻

`target` は `{"kind":"node","id":1}`、`{"kind":"edge","id":2}`、`{"kind":"transaction","index":0}` のいずれかです。

## Development

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
cargo run --release -- render-example calibration out
```

## License

MIT. See [LICENSE](LICENSE).
