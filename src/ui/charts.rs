pub fn graph(
    values: &[Option<f64>],
    width: usize,
    height: usize,
) -> Vec<String> {
    if !values.iter().any(Option::is_some) {
        return std::iter::once("N/A — no samples".into())
            .chain(std::iter::repeat_with(String::new))
            .take(height)
            .collect();
    }
    let count = width.min(values.len()).max(1);
    let series = (0..width.min(values.len()))
        .map(|column| {
            let start = column * values.len() / count;
            let end = ((column + 1) * values.len() / count).max(start + 1);
            values
                .iter()
                .skip(start)
                .take(end.saturating_sub(start))
                .copied()
                .try_fold(None::<f64>, |maximum, value| {
                    value.map(|value| Some(maximum.map_or(value, |maximum| maximum.max(value))))
                })
                .flatten()
        })
        .collect::<Vec<_>>();
    let low = series
        .iter()
        .flatten()
        .copied()
        .reduce(f64::min)
        .map(|value| (value / 10.0).floor() * 10.0)
        .unwrap_or(0.0);
    let high = series
        .iter()
        .flatten()
        .copied()
        .reduce(f64::max)
        .map(|value| ((value / 10.0).ceil() * 10.0).max(low + 10.0))
        .unwrap_or(100.0);
    (0..height)
        .map(|row| {
            let level = high - (high - low) * row as f64 / height.saturating_sub(1).max(1) as f64;
            let pixels = series
                .iter()
                .map(|value| match value {
                    Some(value) if *value >= level => '█',
                    Some(_) => ' ',
                    None => '·',
                })
                .collect::<String>();
            format!(
                "{level:5.0} ┤{}{}",
                " ".repeat(width.saturating_sub(series.len())),
                pixels
            )
        })
        .collect()
}
