# frp-visualizer

[![CI](https://github.com/39sho/frp-visualizer/actions/workflows/ci.yml/badge.svg)](https://github.com/39sho/frp-visualizer/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

FRP（Functional Reactive Programming）の **Stream / Cell ネットワークをテキストで定義し、同じ定義からネットワーク図とイベント伝搬タイミングを生成する CLI ツール**です。

コアが保証するのは **配線・トポロジー・イベントの発火時刻/回数** です。値計算・C++ の副作用・実時間は、必要に応じて実行バックエンドに委ねる設計です。

![Calibration](docs/examples/calibration.svg)

## Features

- `map` / `filter` / `merge` / `snapshot` / `hold` / `lift` などの FRP プリミティブ
- Stream と Cell の型付きポート、離散トランザクションによるシミュレーション
- `network.svg` / `timing.svg` / `diagram.svg` / `diagram.json` の出力
- **orthogonal 配線**（デフォルト）と `--route hybrid` の斜め配線
- ノードとラベルを避けるルーティング、直交配線の交差ペナルティ、平行線の分離、交差部のホップ
- **port-aware auto layout**（直列ノードを入出力ポートの高さに揃え、Sink も入力元に追従）
- Cell を帯で示すタイミング図、トランザクション開始／終了ティック
- **Component 雲**（入出力ポート・多出力・内部ネットワークの展開表示・内部状態を含むシミュレーション）
- **複数ファイル分割**（`import` でコンポーネント定義を読み込み、より大きな Component を組み立て）
- Sink / Component 出力の lane 化と、Component 内部スコープの timing 生成
- 値を出さない **event-only timing**（`--timing-values none`）
- focus した Transaction の **active サブグラフを timing 内に表示**
- `trace.json`（Transaction / lane / event / value / timing reliability）
- `check --strict` と render 時の検証による欠落入力・不正接続の検出
- Node / Edge / Transaction に紐づく**注釈吹き出し**
- Mermaid 風の**テキスト DSL** から JSON への一方向コンパイル
- 構造スケッチ用の **structural mode** と表示専用 `display` ラベル
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
# スターター DSL を作成
frp-visualizer init sample.frp

# 組み込み example の一覧
frp-visualizer list

# JSON として書き出す
frp-visualizer example accumulator accumulator.json

# DSL を JSON にコンパイル
frp-visualizer compile examples/calibration.frp calibration.json

# network.svg / timing.svg / diagram.svg / diagram.json を出力
frp-visualizer render accumulator.json out
frp-visualizer render examples/calibration.frp out

# 組み込み example を直接出力
frp-visualizer render-example calibration out --auto-layout

# トランザクション T2 をフォーカス
frp-visualizer render-example calibration out --focus-transaction 2

# Component の内部ネットワークも出力
frp-visualizer render-example component out --expand accumulate
frp-visualizer render-example component out --expand-all

# 複数ファイルに分割した Component 定義を読み込んで出力
frp-visualizer render examples/modular/main.frp out

# 全体＋内部スコープの timing を生成
frp-visualizer render examples/component.frp out --timing-scope all

# focus した Transaction の active サブグラフを表示
frp-visualizer render examples/component.frp out --focus-transaction 1

# イベント時刻だけを表示（値は出さない）
frp-visualizer render examples/structural.frp out --timing-values none
```

出力先を省略すると `frp_export/` に書き出します。

## CLI

```text
frp-visualizer list
frp-visualizer example <name> [out.json]
frp-visualizer init [out.frp] [--force]
frp-visualizer compile <diagram.frp> [out.json] [--auto-layout]
frp-visualizer check <diagram.frp|diagram.json> [--strict]
frp-visualizer render <diagram.json|diagram.frp> [out_dir] [options]
frp-visualizer version
frp-visualizer render-example <name> [out_dir] [options]
```

| Option | Description |
| --- | --- |
| `--auto-layout` | ノード位置を自動計算 |
| `--route orthogonal` | 水平・垂直配線（デフォルト） |
| `--route hybrid` | 斜め配線を許可 |
| `--focus-transaction N` | N 番目（1 始まり）のトランザクションを実行フォーカス |
| `--expand <component>` | 指定 Component の内部ネットワークを `expanded_<name>.svg` / `.json` に出力 |
| `--timing-scope outer|all|component:<name-or-path>` | タイミング図のスコープ。`all` で全体＋全 Component 内部（nested 含む）を出力 |
| `--timing-values values|none` | イベント値を表示するか。`none` は時刻のみ |
| `--timing-network focus|none` | focus 時に active サブグラフを表示するか |
| `--expand-all` | すべての Component 内部ネットワークを出力 |
| `--output all|network` | `network` は network.svg のみ出力（構造スケッチ向け） |

## Scope

このツールの中心は **ネットワークを書く → 図と event-flow timing を生成する → 検証する** ことです。

- 正確に扱う: トポロジー、型付きポート、Transaction ごとの発火有無・回数、Component の入出力
- 明示的に扱う: 構造スケッチ（`mode structural`）と実行モデルの区別、display-only ノード、警告・エラー
- 任意のバックエンドに委ねる: 値計算、C++ の副作用、実時間、完全な値コピー等価性

通常の `map` は値が不定でもイベントを落とさないため、filter を含まないネットワークでは event-flow timing を正確に生成できます。display-only の値が filter に到達する場合は `timing_reliable=false` として警告します。

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
| [`modular/main.frp`](examples/modular/main.frp) | import で分割した Component 定義の組み立て |

`modular/main.frp` はファイル実行例です（`render-example` の組み込み一覧には含まれません）。

### Components

`Component` は入出力ポートを持つ雲として表示され、内部に `Document` を持てます。シミュレーション時は階層をフラット化して内部の `hold` / `snapshot` まで実行します。

![Component](docs/examples/component.svg)

`--expand` で内部ネットワークを単体表示できます。

![Expanded component](docs/examples/expanded-accumulate.svg)

### Modular component files

コンポーネント定義はファイル単位で分割できます。小さな部品を import して、より大きな Component を組み立てられます。

`components/accumulator.frp`:

```text
component accumulator {
  in delta: stream
  out total: stream
  out state: cell

  snapshot snap = add
  hold accum = 0
  delta -> snap.event
  accum -> snap.cell
  snap -> accum
  snap -> total
  accum -> state
}
```

`components/pipeline.frp`:

```text
import "accumulator.frp"

component pipeline {
  in delta: stream
  out total: stream
  out state: cell
  component acc = accumulator
  delta -> acc.delta
  acc.total -> total
  acc.state -> state
}
```

`main.frp`:

```text
import "components/pipeline.frp"
stream sDelta events 1:5, 2:10
component pipeline = pipeline
map observed = identity
sink out
sDelta -> pipeline.delta
pipeline.total -> observed
observed -> out
```

import は再帰的に解決され、同じファイルが複数回読み込まれても定義は一度だけ登録されます。循環 import、欠落ファイル、定義名の重複はエラーになります。`compile` で書き出した JSON には import 済みの Component 定義がすべて埋め込まれるため、JSON 単体で render できます。

完全な例: [`examples/modular/main.frp`](examples/modular/main.frp)

### Annotations

注釈は Node / Edge / Transaction を対象に、緑の吹き出しと楔形ポインタで描画されます。

![Annotations](docs/examples/annotations.svg)

### Timing scopes

タイミング図は flatten 済みの実行 trace から生成されます。`--timing-scope all` で全体に加えて各 Component の内部 timing を `timing_<component>.svg` に出力します。nested Component は `outer/inner` のようなパスで区別され、`--timing-scope component:outer/inner` で個別に指定できます。

```sh
frp-visualizer render examples/component.frp out --timing-scope all --timing-values none
```

`--timing-values none` は値を出さず、イベントの発火時刻と件数だけを表示します。`--focus-transaction N` と組み合わせると、その Transaction で active なノードと接続だけを timing 内のインセットに表示します。値・副作用の計算は行いません。

`render` は検証エラーがある文書を書き出しません。`check --strict` と併用してください。`trace.json` には `timing_reliable` / `values_reliable`、Transaction ごとの events / event_order / cell updates / update_order、lane ごとの event/step が入ります。

### Execution focus

![Execution](docs/examples/execution.svg)

## DSL

JSON は IR / 外部ツール連携用で、手で管理するなら Mermaid 風の DSL を推奨します。**DSL → JSON の一方向変換**です。

```text
diagram Calibration
time_end 6
layout auto

stream sClearAccumulator events 3:()
stream sDelta events 1:5, 2:10, 4:-3, 5:8
cell calibration = 1 changes 4:2

map clearZero = const 0
snapshot total = add
merge merged
hold accum = 0
lift scaled = mul
sink out

sClearAccumulator -> clearZero
clearZero -> merged.left
sDelta -> total.event
accum -> total.cell
total -> merged.right
merged -> accum
accum -> scaled.a
calibration -> scaled.b
scaled -> out
```

```sh
frp-visualizer check examples/calibration.frp --strict
frp-visualizer compile examples/calibration.frp calibration.json
frp-visualizer render examples/calibration.frp out
frp-visualizer render examples/structural.frp structural-out --output network
```

- `layout auto` / `layout manual`、ノードごとの `at x,y` に対応。auto layout は Sink を最右列に置き、入力側の重心に合わせて配置するため、大きな図でも `out` への配線が戻りにくい
- `stream` / `cell` / `map` / `filter` / `merge` / `snapshot` / `hold` / `lift` / `never` / `sink`
- `component <name> { ... }` で Component を定義し、`component <instance> = <definition>` で配置
- `import "path/to/component.frp"` で別ファイルの Component 定義を読み込み、現在のファイルから参照可能にする
- `import` はファイル先頭（コメント・空行を除く）に置く。パスはそのファイルからの相対パス
- import されるファイルは `import` と `component` 定義だけを含める。定義名はプロジェクト全体で一意
- ポートは `merged.left`、`total.event`、`component.port` のように指定
- `note "..." on <node>` / `note "..." on tx <N>` で注釈
- `mode structural` で構造スケッチ用のモードに切り替え
- `map update = display "SetReferenceMapPoints"` のように表示専用の処理名を指定
- エッジラベルは `\n` で改行でき、`label @ 240` で幅を指定（JSON では `label_width`）
- Component は定義を先に書き、その後にインスタンスを配置

`mode structural` と `display` を使うと、実行意味論を持たない構造スケッチを描けます。値は timing に出ません。

![Structural mode](docs/examples/structural.svg)

完全な例は [`examples/calibration.frp`](examples/calibration.frp)、[`examples/component.frp`](examples/component.frp)、[`examples/annotation.frp`](examples/annotation.frp)、[`examples/structural.frp`](examples/structural.frp)、[`examples/nested.frp`](examples/nested.frp)、[`examples/modular/main.frp`](examples/modular/main.frp) にあります。

## JSON format

JSON はコンパイル済みの IR です。`frp-visualizer example accumulator` で完全な例を確認できます。主なフィールド:

- `nodes`: `id` / `kind` / `name` / `pos` / `label`
- `edges`: `from` / `from_port` / `to` / `to_port` / `label` / `label_width`
- `components`: `name` / `inputs` / `outputs` / `body` / `doc`（import された定義も含む）
- `annotations`: `text` / `target` / `position`
- `nodes[].display_only`: 実行意味論を持たない表示専用ノード
- `mode`: `executable`（デフォルト）または `structural`
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
