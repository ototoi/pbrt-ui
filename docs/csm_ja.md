# CSM アルゴリズム設計メモ（日本語）

この文書は、現在の実装方針として合意した CSM（Cascaded Shadow Maps）アルゴリズムを整理したものです。

## 前提と方針

- `split` はカメラ方向（カメラ空間の深度）で扱う。
- `split_min`, `split_max` は **receiver のカメラ空間深度範囲**を用いて決める。
- カスケードは `split_min..split_max` を `cascade_count` で分割する。
- 分割は `linear/log` を `CASCADE_SPLIT_LAMBDA` で補間する。
- 各カスケードで、`receiver` と `caster` を別役割で使う:
  - `XY` は receiver から作る
  - `Z` は caster から作る
- **フォールバックはしない**。前提が壊れたら `assert!` で停止する。

## 用語

- `camera_min`, `camera_max`: カメラ near/far（正の深度）
- `receiver_min`, `receiver_max`: receiver 点群をカメラ空間へ投影した深度範囲（正）
- `split_min`, `split_max`:
  - `split_min = max(camera_min, receiver_min)`
  - `split_max = min(camera_max, receiver_max)`

## 手順

1. カメラ空間の深度範囲を計算する
- `camera_min`, `camera_max` を取得
- `receiver_min`, `receiver_max` を計算
- 前提:
  - `camera_min > 0`, `camera_max > 0`, `camera_min < camera_max`
  - `receiver_min > 0`, `receiver_max > 0`, `receiver_min < receiver_max`

2. split 範囲を確定する
- `split_min = max(camera_min, receiver_min)`
- `split_max = min(camera_max, receiver_max)`
- `split_min > EPSILON` を保証する（必要なら clamp）
- `split_max > split_min` を保証する

3. カスケード分割値を作る
- `split_min..split_max` を `cascade_count` 分割
- `CASCADE_SPLIT_LAMBDA` で `linear/log` 補間

4. 各カスケードごとに receiver フラスタムスライスを作る
- カメラフラスタムを、当該カスケードの near/far で切る
- スライスの 8 点を得る（receiver スライス）

5. 各カスケードごとに light_view 空間で receiver の XY を作る
- スライス 8 点を `light_view` に投影
- 投影点から `XY` の AABB を作る

6. 各カスケードごとに caster から Z 範囲を作る
- receiver の `XY` フットプリントと交差する caster を集める（まずは AABB 交差で可）
- 交差 caster から `Zmin/Zmax` を計算
- これをそのカスケードの `Z` とする（スライス共通ではなく、スライスごと）

7. 射影行列を作る
- 各カスケードで `XY`（receiver）と `Z`（caster）から `light_proj` を作る
- `light_view_proj = light_proj * light_view`

8. シェーダ側 split_end 対応
- `split_end` はカスケード選択に使う値として、カメラ空間深度系と整合する値を使う
- `project_directional_shadow_uv_depth` と `get_directional_shadow_cascade_index` の前提と一致させる

## 交差判定（caster 取得）方針

- 理想は「receiver の XY ポリゴン」との交差判定
- ただし初期実装は簡易化のため AABB ベースで可
- 品質が不足する場合にポリゴン判定へ拡張する

## 検証ルール（assert）

フォールバックは行わないため、各段階で必ず `assert!` する:

- split 範囲:
  - `split_min.is_finite() && split_max.is_finite()`
  - `split_min > EPSILON`
  - `split_max > split_min`
- スライス 8 点:
  - 全点 finite
- receiver XY:
  - `x_max > x_min`, `y_max > y_min`
- caster Z:
  - `z_max > z_min`
- 射影パラメータ:
  - `near > 0`, `far > near`

破綻時はアルゴリズム不整合として即停止し、値をログ出力して原因を特定する。
