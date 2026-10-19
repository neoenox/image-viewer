//! 表示レイテンシの計測（F12 オーバーレイ）。
//!
//! `loader::bench` はワーカーが画像を作り終えるまでしか測らず、描画と
//! テクスチャのアップロードは含まない（README の「Latency benchmark」を参照）。
//! ここでは利用者が実際に待つ値を測る：
//!
//! - フレーム時間（1フレームの所要時間）
//! - 選択 → 表示（画像を要求してからテクスチャが画面に出るまで）
//! - テクスチャ生成（`load_texture` の所要時間＝アップロード）
//!
//! 計測は `show_stats` が有効なときだけ動かす。無効時は `Instant` を取ることすら
//! しないので、アイドル時のコストは変わらない。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// 保持するサンプル数。有界にして、計測自体がメモリを増やさないようにする。
pub(crate) const SAMPLE_WINDOW: usize = 120;

/// 直近 [`SAMPLE_WINDOW`] 件のサンプルだけを保持する。
fn push_bounded(samples: &mut VecDeque<f32>, value: f32) {
    if samples.len() == SAMPLE_WINDOW {
        samples.pop_front();
    }
    samples.push_back(value);
}

/// サンプルの分位点（最近傍順位法）。空なら `None`。
fn percentile(samples: &VecDeque<f32>, q: f32) -> Option<f32> {
    if samples.is_empty() {
        return None;
    }
    let mut values: Vec<f32> = samples.iter().copied().collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let index = ((values.len() - 1) as f32 * q.clamp(0.0, 1.0)).round() as usize;
    Some(values[index.min(values.len() - 1)])
}

/// 中央値。偶数個は中央2つの平均（最近傍順位法だと 1..=100 の中央値が
/// 50 ではなく 51 になるため、代表値としての中央値はこちらで出す）。
fn median(samples: &VecDeque<f32>) -> Option<f32> {
    if samples.is_empty() {
        return None;
    }
    let mut values: Vec<f32> = samples.iter().copied().collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        Some((values[mid - 1] + values[mid]) / 2.0)
    } else {
        Some(values[mid])
    }
}

fn millis(duration: Duration) -> f32 {
    duration.as_secs_f32() * 1000.0
}

/// フレーム・表示・アップロードの実測値。`Default` は「何も測っていない」。
#[derive(Default)]
pub struct LatencyStats {
    frame_samples: VecDeque<f32>,
    display_samples: VecDeque<f32>,
    upload_samples: VecDeque<f32>,
    frame_start: Option<Instant>,
    /// 現在画像の読み込みを要求した時刻。結果が表示された時点で消費する。
    selection_at: Option<Instant>,
    last_display_ms: Option<f32>,
    last_upload_ms: Option<f32>,
    frames: u64,
}

impl LatencyStats {
    pub(crate) fn begin_frame(&mut self, now: Instant) {
        self.frame_start = Some(now);
    }

    /// フレームを閉じ、その所要時間（ms）を記録して返す。
    pub(crate) fn end_frame(&mut self, now: Instant) -> Option<f32> {
        let start = self.frame_start.take()?;
        let ms = millis(now.saturating_duration_since(start));
        self.frames += 1;
        push_bounded(&mut self.frame_samples, ms);
        Some(ms)
    }

    /// 新しい画像の読み込みを要求した。表示されるまで1回だけ数える。
    pub(crate) fn note_selection(&mut self, now: Instant) {
        self.selection_at = Some(now);
    }

    /// 要求した画像が画面に出た。選択からの経過時間を1サンプルとして記録する。
    /// 要求していない（先読み・GIF のフレームなど）場合は何もしない。
    pub(crate) fn note_displayed(&mut self, now: Instant) {
        if let Some(at) = self.selection_at.take() {
            let ms = millis(now.saturating_duration_since(at));
            self.last_display_ms = Some(ms);
            push_bounded(&mut self.display_samples, ms);
        }
    }

    /// 読み込みが失敗した／破棄された。要求を閉じて、誤った待ち時間を残さない。
    pub(crate) fn clear_pending(&mut self) {
        self.selection_at = None;
    }

    /// テクスチャ生成（アップロード）の所要時間。
    pub(crate) fn note_upload(&mut self, elapsed: Duration) {
        let ms = millis(elapsed);
        self.last_upload_ms = Some(ms);
        push_bounded(&mut self.upload_samples, ms);
    }

    pub(crate) fn frames(&self) -> u64 {
        self.frames
    }

    pub(crate) fn frame_median_ms(&self) -> Option<f32> {
        median(&self.frame_samples)
    }

    pub(crate) fn frame_p95_ms(&self) -> Option<f32> {
        percentile(&self.frame_samples, 0.95)
    }

    pub(crate) fn display_median_ms(&self) -> Option<f32> {
        median(&self.display_samples)
    }

    pub(crate) fn display_last_ms(&self) -> Option<f32> {
        self.last_display_ms
    }

    pub(crate) fn upload_last_ms(&self) -> Option<f32> {
        self.last_upload_ms
    }

    pub(crate) fn upload_p95_ms(&self) -> Option<f32> {
        percentile(&self.upload_samples, 0.95)
    }

    pub(crate) fn display_sample_count(&self) -> usize {
        self.display_samples.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u32) -> Duration {
        Duration::from_millis(u64::from(value))
    }

    #[test]
    fn frame_samples_are_bounded_and_keep_the_newest() {
        let mut stats = LatencyStats::default();
        let base = Instant::now();
        for i in 0..(SAMPLE_WINDOW + 10) {
            stats.begin_frame(base);
            stats.end_frame(base + ms(i as u32));
        }
        assert_eq!(stats.frames(), (SAMPLE_WINDOW + 10) as u64);
        assert_eq!(stats.frame_samples.len(), SAMPLE_WINDOW);
        // 最古の10件は落ち、新しい方が残る。
        assert_eq!(stats.frame_samples.front().copied(), Some(10.0));
        assert_eq!(
            stats.frame_samples.back().copied(),
            Some((SAMPLE_WINDOW + 9) as f32)
        );
    }

    #[test]
    fn end_frame_without_begin_is_ignored() {
        let mut stats = LatencyStats::default();
        assert_eq!(stats.end_frame(Instant::now()), None);
        assert_eq!(stats.frames(), 0);
        assert!(stats.frame_median_ms().is_none());
    }

    #[test]
    fn percentile_reports_median_and_p95() {
        let samples: VecDeque<f32> = (1..=100).map(|v| v as f32).collect();
        // 偶数個の中央値は中央2つの平均、p95 は最近傍順位法。
        assert_eq!(median(&samples), Some(50.5));
        assert_eq!(percentile(&samples, 0.95), Some(95.0));
        let odd: VecDeque<f32> = (1..=5).map(|v| v as f32).collect();
        assert_eq!(median(&odd), Some(3.0));
        assert_eq!(median(&VecDeque::new()), None);
        assert_eq!(percentile(&VecDeque::new(), 0.95), None);
    }

    #[test]
    fn selection_to_display_is_measured_once_per_request() {
        let mut stats = LatencyStats::default();
        let base = Instant::now();
        stats.note_selection(base);
        stats.note_displayed(base + ms(40));
        assert_eq!(stats.display_last_ms(), Some(40.0));
        assert_eq!(stats.display_sample_count(), 1);
        // 同じ要求を二度数えない（表示直後のフレームで二重計上しない）。
        stats.note_displayed(base + ms(80));
        assert_eq!(stats.display_sample_count(), 1);
        assert_eq!(stats.display_last_ms(), Some(40.0));
    }

    #[test]
    fn display_without_a_pending_request_is_ignored() {
        let mut stats = LatencyStats::default();
        // 先読みや GIF フレームの適用は「選択 → 表示」ではない。
        stats.note_displayed(Instant::now());
        assert_eq!(stats.display_sample_count(), 0);
        assert_eq!(stats.display_last_ms(), None);
    }

    #[test]
    fn clear_pending_drops_the_waiting_request() {
        let mut stats = LatencyStats::default();
        let base = Instant::now();
        stats.note_selection(base);
        stats.clear_pending();
        stats.note_displayed(base + ms(500));
        assert_eq!(stats.display_sample_count(), 0);
        assert_eq!(stats.display_last_ms(), None);
    }

    #[test]
    fn upload_samples_are_recorded_and_bounded() {
        let mut stats = LatencyStats::default();
        assert_eq!(stats.upload_last_ms(), None);
        for i in 0..(SAMPLE_WINDOW + 5) {
            stats.note_upload(ms(i as u32));
        }
        assert_eq!(stats.upload_samples.len(), SAMPLE_WINDOW);
        assert_eq!(stats.upload_last_ms(), Some((SAMPLE_WINDOW + 4) as f32));
        // 単調増加なので p95 は最新付近、最大は含まれない。
        assert!(stats.upload_p95_ms().unwrap() < (SAMPLE_WINDOW + 4) as f32);
    }
}
