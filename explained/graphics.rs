// =============================================================================
// graphics.rs - グラフィック描画機能モジュール
// =============================================================================
//
// このファイルは、OSの低レベルグラフィック描画機能を提供するモジュールです。
// UEFI環境でのVRAM（ビデオRAM）への直接描画を抽象化し、
// 安全で使いやすいインターフェースを提供します。

use crate::result::Result;  // カスタムResult型をインポート
use core::cmp::min;         // 最小値を求める関数

// =============================================================================
// Bitmapトレイト - 描画対象の抽象化
// =============================================================================

/// 描画可能なビットマップを表すトレイト
/// 
/// このトレイトは、VRAM、フレームバッファ、その他のビットマップに対して
/// 統一的な描画インターフェースを提供します。
/// 
/// # 設計思想
/// - ハードウェア抽象化: 実際の描画先（VRAM等）の詳細を隠蔽
/// - 型安全性: ジェネリクスにより、コンパイル時に型チェック
/// - パフォーマンス: unsafe関数により高速アクセスも提供
pub trait Bitmap {
    /// 1ピクセルあたりのバイト数を返す
    /// 
    /// 通常のRGBA形式では4バイト（各色8bit + Alpha8bit）
    fn bytes_per_pixel(&self) -> i64;
    
    /// 1スキャンラインあたりのピクセル数を返す
    /// 
    /// 注意: 実際の画面幅より大きい場合があります
    /// （メモリアライメントやハードウェア制約のため）
    fn pixels_per_scan_line(&self) -> i64;
    
    /// 画面の幅（ピクセル単位）
    fn width(&self) -> i64;
    
    /// 画面の高さ（ピクセル単位）
    fn height(&self) -> i64;
    
    /// フレームバッファの先頭ポインタを取得（可変参照）
    /// 
    /// # Safety
    /// 返されるポインタは生ポインタです。使用時は範囲チェックが必要。
    fn buf_mut(&mut self) -> *mut u8;

    /// 指定座標のピクセルアドレスを取得（範囲チェックなし）
    /// 
    /// # Safety
    /// - 座標(x, y)が有効範囲内である前提で動作
    /// - is_in_x_range, is_in_y_rangeで事前チェック済みであることが前提
    /// - 範囲外アクセスは未定義動作を引き起こします
    /// 
    /// # パフォーマンス
    /// 範囲チェックを省略するため高速ですが、安全性は呼び出し側に委ねられます
    unsafe fn unchecked_pixel_at_mut(&mut self, x: i64, y: i64) -> *mut u32 {
        self.buf_mut()
            .add(((y * self.pixels_per_scan_line() + x) * self.bytes_per_pixel()) as usize)
            as *mut u32
    }
    
    /// 指定座標のピクセルアドレスを安全に取得
    /// 
    /// # 戻り値
    /// - Some(pointer): 座標が有効な場合
    /// - None: 座標が範囲外の場合
    /// 
    /// # 使用例
    /// ```rust
    /// if let Some(pixel) = bitmap.pixel_at_mut(100, 50) {
    ///     unsafe { *pixel = 0xFF0000; } // 赤色で塗る
    /// }
    /// ```
    fn pixel_at_mut(&mut self, x: i64, y: i64) -> Option<*mut u32> {
        if self.is_in_x_range(x) && self.is_in_y_range(y) {
            // SAFETY: (x, y)は上記のチェックで有効性が確認済み
            unsafe { Some(&mut *self.unchecked_pixel_at_mut(x, y)) }
        } else {
            None
        }
    }
    
    /// X座標が有効範囲内かチェック
    /// 
    /// 実際の画面幅とスキャンライン幅の小さい方を上限とする
    fn is_in_x_range(&self, px: i64) -> bool {
        0 <= px && px < min(self.width(), self.pixels_per_scan_line())
    }
    
    /// Y座標が有効範囲内かチェック
    fn is_in_y_range(&self, py: i64) -> bool {
        0 <= py && py < self.height()
    }
}

// =============================================================================
// 基本描画関数群
// =============================================================================

/// 1点描画（範囲チェックなし）
/// 
/// # Safety
/// (x, y)が有効な座標である前提で動作します。
/// 範囲外アクセスは未定義動作を引き起こします。
/// 
/// # 用途
/// ループ内での大量の点描画で、事前に範囲チェック済みの場合に使用
unsafe fn unchecked_draw_point<T: Bitmap>(buf: &mut T, color: u32, x: i64, y: i64) {
    *buf.unchecked_pixel_at_mut(x, y) = color;
}

/// 1点描画（安全版）
/// 
/// # 戻り値
/// - Ok(()): 描画成功
/// - Err("Out of Range"): 座標が範囲外
/// 
/// # 色形式
/// color: 0xRRGGBB形式（24bit RGB）
fn draw_point<T: Bitmap>(buf: &mut T, color: u32, x: i64, y: i64) -> Result<()> {
    unsafe {
        *(buf.pixel_at_mut(x, y).ok_or("Out of Range")?) = color;
    }
    Ok(())
}

/// 矩形塗りつぶし
/// 
/// # 引数
/// - buf: 描画先ビットマップ
/// - color: 塗りつぶし色（0xRRGGBB形式）
/// - px, py: 左上角の座標
/// - w, h: 幅、高さ
/// 
/// # エラー処理
/// 矩形の一部でも範囲外に出る場合はエラーを返し、何も描画しません
/// 
/// # パフォーマンス最適化
/// 事前に範囲チェックを行い、内部ループではunsafe関数を使用して高速化
pub fn fill_rect<T: Bitmap>(buf: &mut T, color: u32, px: i64, py: i64, w: i64, h: i64) -> Result<()> {
    // 矩形全体が範囲内かチェック
    if !buf.is_in_x_range(px)
        || !buf.is_in_y_range(py)
        || !buf.is_in_x_range(px + w - 1)  // 右端
        || !buf.is_in_y_range(py + h - 1)  // 下端
    {
        return Err("Out of Range");
    }
    
    // 範囲チェック済みなのでunsafe関数を使用して高速化
    for y in py..py + h {
        for x in px..px + w {
            unsafe {
                unchecked_draw_point(buf, color, x, y);
            }
        }
    }
    Ok(())
}

/// 直線描画のための勾配計算
/// 
/// ブレゼンハムアルゴリズムの変形を使用
/// 
/// # 引数
/// - da: 主軸方向の差分
/// - db: 副軸方向の差分  
/// - ia: 主軸における現在位置
/// 
/// # 戻り値
/// - Some(value): 副軸における対応位置
/// - None: 計算不可能な場合
/// 
/// # アルゴリズム
/// 整数演算のみで直線の勾配を近似計算
fn calc_slope_point(da: i64, db: i64, ia: i64) -> Option<i64> {
    if da < db {
        None  // 主軸と副軸が逆転している
    } else if da == 0 {
        Some(0)  // 差分がない場合
    } else if (0..=da).contains(&ia) {
        // 線形補間による勾配計算（整数演算）
        Some((2 * db * ia + da) / da / 2)
    } else {
        None  // 範囲外
    }
}

/// 直線描画
/// 
/// # アルゴリズム
/// デジタル微分解析器（DDA）のような手法で直線を描画
/// X軸とY軸の差分の大きい方を主軸として選択し、滑らかな線を描画
/// 
/// # 引数
/// - (x0, y0): 開始点
/// - (x1, y1): 終了点
/// - color: 線の色
/// 
/// # 注意
/// 現在は内部関数（非公開）ですが、将来的に公開される可能性があります
fn draw_line<T: Bitmap>(buf: &mut T, color: u32, x0: i64, y0: i64, x1: i64, y1: i64) -> Result<()> {
    // 両端点が範囲内かチェック
    if !buf.is_in_x_range(x0)
        || !buf.is_in_y_range(y0)
        || !buf.is_in_x_range(x1)
        || !buf.is_in_y_range(y1)
    {
        return Err("Out of Range");
    }
    
    // 差分と方向を計算
    let dx = (x1 - x0).abs();  // X方向の絶対距離
    let dy = (y1 - y0).abs();  // Y方向の絶対距離
    let sx = (x1 - x0).signum();  // X方向の単位ベクトル (-1, 0, 1)
    let sy = (y1 - y0).signum();  // Y方向の単位ベクトル (-1, 0, 1)
    
    // 主軸を決定してブレゼンハム風アルゴリズムで描画
    if dx >= dy {
        // X軸が主軸の場合
        for (rx, ry) in (0..dx).flat_map(|rx| calc_slope_point(dx, dy, rx).map(|ry| (rx, ry))) {
            draw_point(buf, color, x0 + rx * sx, y0 + ry * sy)?;
        }
    } else {
        // Y軸が主軸の場合
        for (rx, ry) in (0..dy).flat_map(|ry| calc_slope_point(dy, dx, ry).map(|rx| (rx, ry))) {
            draw_point(buf, color, x0 + rx * sx, y0 + ry * sy)?;
        }
    }
    Ok(())
}

// =============================================================================
// フォント描画システム
// =============================================================================

/// フォントデータ検索
/// 
/// # フォント形式
/// - 8x16ピクセルのビットマップフォント
/// - テキストファイル（font.txt）から動的ロード
/// - '*'文字で描画ピクセル、その他は透明として扱う
/// 
/// # 引数
/// - c: 描画したい文字
/// 
/// # 戻り値
/// - Some(bitmap): フォントビットマップ（8x16の2次元配列）
/// - None: 対応するフォントが見つからない場合
/// 
/// # フォントファイル形式
/// ```
/// 0x41    # ASCII 'A'の例
/// ********
/// *      *
/// *      *
/// ********
/// ...（16行）
/// ```
fn lookup_font(c: char) -> Option<[[char; 8]; 16]> {
    const FONT_SOURCE: &str = include_str!("font.txt");  // コンパイル時にファイル埋め込み
    
    if let Ok(c) = u8::try_from(c) {  // ASCII文字のみサポート
        let mut fi = FONT_SOURCE.split('\n');
        
        // フォントファイルを行ごとに解析
        while let Some(line) = fi.next() {
            if let Some(line) = line.strip_prefix("0x") {  // "0x"で始まる行を探す
                if let Ok(idx) = u8::from_str_radix(line, 16) {  // 16進数解析
                    if idx != c {
                        continue;  // 目的の文字ではない
                    }
                    
                    // 8x16のビットマップを構築
                    let mut font = [['*'; 8]; 16];  // デフォルトは全て塗りつぶし
                    for (y, line) in fi.clone().take(16).enumerate() {  // 次の16行を取得
                        for (x, c) in line.chars().enumerate() {
                            if let Some(e) = font[y].get_mut(x) {
                                *e = c;
                            }
                        }
                    }
                    return Some(font);
                }
            }
        }
    }
    None  // フォントが見つからない
}

/// 単一文字描画（前景色のみ）
/// 
/// # 引数
/// - buf: 描画先
/// - x, y: 描画位置（左上角）
/// - color: 文字色
/// - c: 描画する文字
/// 
/// # 描画仕様
/// - 文字サイズ: 8x16ピクセル
/// - 透明部分は描画しない（背景色を変更しない）
/// - フォントが見つからない場合は何も描画しない
/// 
/// # 使用例
/// 通常は draw_str_fg から呼び出されます
pub fn draw_font_fg<T: Bitmap>(buf: &mut T, x: i64, y: i64, color: u32, c: char) {
    if let Some(font) = lookup_font(c) {
        // フォントビットマップを走査
        for (dy, row) in font.iter().enumerate() {
            for (dx, pixel) in row.iter().enumerate() {
                let color = match pixel {
                    '*' => color,      // 描画ピクセル
                    _ => continue,     // 透明ピクセル（スキップ）
                };
                // エラーは無視（範囲外でも描画を継続）
                let _ = draw_point(buf, color, x + dx as i64, y + dy as i64);
            }
        }
    }
}

/// 文字列描画（前景色のみ）
/// 
/// # 引数
/// - buf: 描画先
/// - x, y: 開始位置
/// - color: 文字色
/// - s: 描画する文字列
/// 
/// # 描画仕様
/// - 文字間隔: 8ピクセル（固定幅フォント）
/// - 改行制御なし（1行のみ）
/// - 範囲外の文字は自動的にスキップ
fn draw_str_fg<T: Bitmap>(buf: &mut T, x: i64, y: i64, color: u32, s: &str) {
    for (i, c) in s.chars().enumerate() {
        draw_font_fg(buf, x + i as i64 * 8, y, color, c);
    }
}

// =============================================================================
// テストパターン描画
// =============================================================================

/// デバッグ用テストパターン描画
/// 
/// # 描画内容
/// 1. カラーバー: 黒、赤、緑、青の矩形
/// 2. 反転カラーバー: 各色の補色
/// 3. 対角線: 白い線で接続
/// 4. テキスト: "0123456789" と "ABCDEF"
/// 
/// # 配置
/// 画面右端に128x256ピクセルの領域に描画
/// 
/// # 用途
/// - グラフィックシステムの動作確認
/// - 色表示の正確性テスト
/// - フォント描画の確認
/// - 座標系の確認
pub fn draw_test_pattern<T: Bitmap>(buf: &mut T) {
    let w = 128;                           // テストパターンの幅
    let left = buf.width() - w - 1;        // 右端からの開始位置
    let colors = [0x000000, 0xff0000, 0x00ff00, 0x0000ff];  // 黒、赤、緑、青
    let h = 64;                            // 各カラーバーの高さ
    
    // カラーバーとその反転色を描画
    for (i, c) in colors.iter().enumerate() {
        let y = i as i64 * h;
        fill_rect(buf, *c, left, y, h, h).expect("fill_rect failed");      // 元の色
        fill_rect(buf, !*c, left + h, y, h, h).expect("fill_rect failed"); // 反転色
    }
    
    // 対角線を描画（接続性テスト）
    let points = [(0, 0), (0, w), (w, 0), (w, w)];  // 四隅の点
    for (x0, y0) in points.iter() {
        for (x1, y1) in points.iter() {
            let _ = draw_line(buf, 0xffffff, left + *x0, *y0, left + *x1, *y1);
        }
    }
    
    // テストテキストを描画（フォントシステムテスト）
    draw_str_fg(buf, left, h * colors.len() as i64, 0x00ff00, "0123456789");
    draw_str_fg(buf, left, h * colors.len() as i64 + 16, 0x00ff00, "ABCDEF");
}

// =============================================================================
// モジュール設計思想
// =============================================================================
//
// 1. 抽象化レベル
//    - Bitmapトレイト: ハードウェア非依存の描画インターフェース
//    - 基本図形関数: 点、線、矩形などの基本要素
//    - フォントシステム: テキスト描画の高レベルAPI
//
// 2. 安全性
//    - unsafeな操作は明確にマーク
//    - 範囲チェック付きの安全なAPI
//    - パフォーマンス重視時のunsafe版も提供
//
// 3. 拡張性
//    - トレイトベースの設計により異なるハードウェアに対応
//    - 新しい図形描画関数を容易に追加可能
//    - フォントシステムも拡張可能
//
// 4. エラーハンドリング
//    - Result型による明示的なエラー処理
//    - 部分的な失敗（範囲外描画）にも対応
//    - デバッグとリリース両方を考慮