// =========================================
// =========================================
// crates/motionloom/src/transitions.rs

use crate::SlideDirection;
use std::time::Duration;

pub fn sample_fade_factor(
    duration: Duration,
    fade_in_raw: f32,
    fade_out_raw: f32,
    t: Duration,
) -> f32 {
    let total = duration.as_secs_f32();
    if total <= 0.0 {
        return 1.0;
    }
    let fade_in = fade_in_raw.clamp(0.0, total);
    let fade_out = fade_out_raw.clamp(0.0, total);
    if fade_in <= 0.001 && fade_out <= 0.001 {
        return 1.0;
    }
    let local = t.as_secs_f32().clamp(0.0, total);
    let mut factor: f32 = 1.0;
    if fade_in > 0.001 {
        factor = factor.min((local / fade_in).clamp(0.0, 1.0));
    }
    if fade_out > 0.001 {
        factor = factor.min(((total - local) / fade_out).clamp(0.0, 1.0));
    }
    factor
}

pub fn sample_dissolve_factor(
    duration: Duration,
    dissolve_in_raw: f32,
    dissolve_out_raw: f32,
    t: Duration,
) -> f32 {
    let total = duration.as_secs_f32();
    if total <= 0.0 {
        return 1.0;
    }
    let dissolve_in = dissolve_in_raw.clamp(0.0, total);
    let dissolve_out = dissolve_out_raw.clamp(0.0, total);
    if dissolve_in <= 0.001 && dissolve_out <= 0.001 {
        return 1.0;
    }
    let local = t.as_secs_f32().clamp(0.0, total);
    let mut factor: f32 = 1.0;
    if dissolve_in > 0.001 {
        let in_factor = (local / dissolve_in).clamp(0.0, 1.0);
        factor = factor.min(in_factor.powf(0.4545));
    }
    if dissolve_out > 0.001 {
        let out_factor = ((total - local) / dissolve_out).clamp(0.0, 1.0);
        factor = factor.min(out_factor.powf(0.4545));
    }
    factor
}

pub fn sample_slide_offset(
    duration: Duration,
    in_dir: (f32, f32),
    out_dir: (f32, f32),
    slide_in_raw: f32,
    slide_out_raw: f32,
    t: Duration,
) -> (f32, f32) {
    let total = duration.as_secs_f32();
    if total <= 0.0 {
        return (0.0, 0.0);
    }

    let slide_in = slide_in_raw.clamp(0.0, total);
    let slide_out = slide_out_raw.clamp(0.0, total);
    if slide_in <= 0.001 && slide_out <= 0.001 {
        return (0.0, 0.0);
    }

    let local = t.as_secs_f32().clamp(0.0, total);
    let mut offset_x = 0.0;
    let mut offset_y = 0.0;

    if slide_in > 0.001 && local < slide_in {
        let p = (local / slide_in).clamp(0.0, 1.0);
        let k = 1.0 - p;
        offset_x += in_dir.0 * k;
        offset_y += in_dir.1 * k;
    }

    if slide_out > 0.001 && local > (total - slide_out) {
        let p = ((local - (total - slide_out)) / slide_out).clamp(0.0, 1.0);
        offset_x += out_dir.0 * p;
        offset_y += out_dir.1 * p;
    }

    (offset_x, offset_y)
}

pub fn slide_direction_vector(dir: SlideDirection) -> (f32, f32) {
    match dir {
        SlideDirection::Left => (-1.0, 0.0),
        SlideDirection::Right => (1.0, 0.0),
        SlideDirection::Up => (0.0, -1.0),
        SlideDirection::Down => (0.0, 1.0),
    }
}

pub fn sample_zoom_factor(
    duration: Duration,
    zoom_in_raw: f32,
    zoom_out_raw: f32,
    zoom_amount: f32,
    t: Duration,
) -> f32 {
    let total = duration.as_secs_f32();
    if total <= 0.0 {
        return 1.0;
    }
    let zoom_in = zoom_in_raw.clamp(0.0, total);
    let zoom_out = zoom_out_raw.clamp(0.0, total);
    if zoom_in <= 0.001 && zoom_out <= 0.001 {
        return 1.0;
    }

    let local = t.as_secs_f32().clamp(0.0, total);
    let mut factor = 1.0;
    if zoom_in > 0.001 && local < zoom_in {
        let p = (local / zoom_in).clamp(0.0, 1.0);
        factor = zoom_amount + (1.0 - zoom_amount) * p;
    }
    if zoom_out > 0.001 && local > (total - zoom_out) {
        let p = ((local - (total - zoom_out)) / zoom_out).clamp(0.0, 1.0);
        factor = 1.0 + (zoom_amount - 1.0) * p;
    }
    factor
}

pub fn sample_shock_zoom_factor(
    duration: Duration,
    shock_in_raw: f32,
    shock_out_raw: f32,
    shock_amount: f32,
    t: Duration,
) -> f32 {
    let total = duration.as_secs_f32();
    if total <= 0.0 {
        return 1.0;
    }
    let shock_in = shock_in_raw.clamp(0.0, total);
    let shock_out = shock_out_raw.clamp(0.0, total);
    if shock_in <= 0.001 && shock_out <= 0.001 {
        return 1.0;
    }
    let local = t.as_secs_f32().clamp(0.0, total);
    const SHOCK_DECAY: f32 = 6.0;
    let mut pulse_in = 0.0;
    if shock_in > 0.001 && local < shock_in {
        let p = (local / shock_in).clamp(0.0, 1.0);
        pulse_in = (-(SHOCK_DECAY) * p).exp();
    }
    let mut pulse_out = 0.0;
    if shock_out > 0.001 && local > (total - shock_out) {
        let p = ((local - (total - shock_out)) / shock_out).clamp(0.0, 1.0);
        pulse_out = (-(SHOCK_DECAY) * (1.0 - p)).exp();
    }
    let pulse = pulse_in.max(pulse_out);
    1.0 + (shock_amount - 1.0) * pulse
}

pub fn envelope_factor_at(
    duration: Duration,
    fade_in: Duration,
    fade_out: Duration,
    local_time: Duration,
) -> f32 {
    let total = duration.as_secs_f32().max(0.000_1);
    let local = local_time.as_secs_f32().clamp(0.0, total);
    let fade_in = fade_in.as_secs_f32().clamp(0.0, total);
    let fade_out = fade_out.as_secs_f32().clamp(0.0, total);

    let mut factor: f32 = 1.0;
    if fade_in > 0.000_5 {
        factor = factor.min((local / fade_in).clamp(0.0, 1.0));
    }
    if fade_out > 0.000_5 {
        factor = factor.min(((total - local) / fade_out).clamp(0.0, 1.0));
    }
    factor
}

pub fn build_slide_expr(
    duration: Duration,
    in_dir: SlideDirection,
    out_dir: SlideDirection,
    slide_in_raw: f32,
    slide_out_raw: f32,
    time_var: &str,
) -> (String, String) {
    let duration = duration.as_secs_f64();
    if duration <= 0.0 {
        return ("0".to_string(), "0".to_string());
    }
    let slide_in = (slide_in_raw as f64).max(0.0).min(duration);
    let slide_out = (slide_out_raw as f64).max(0.0).min(duration);
    if slide_in <= 0.001 && slide_out <= 0.001 {
        return ("0".to_string(), "0".to_string());
    }

    let (start_x, start_y) = slide_direction_vector(in_dir);
    let (end_x, end_y) = slide_direction_vector(out_dir);

    let in_expr = if slide_in > 0.001 {
        let p = format!("clip({t}/{d:.6},0,1)", t = time_var, d = slide_in);
        (
            format!("({sx:.3})*(1-({p}))", sx = start_x, p = p),
            format!("({sy:.3})*(1-({p}))", sy = start_y, p = p),
        )
    } else {
        ("0".to_string(), "0".to_string())
    };

    let out_expr = if slide_out > 0.001 {
        let start = (duration - slide_out).max(0.0);
        let p = format!(
            "clip(({t}-{s:.6})/{d:.6},0,1)",
            t = time_var,
            s = start,
            d = slide_out
        );
        (
            format!("({ex:.3})*({p})", ex = end_x, p = p),
            format!("({ey:.3})*({p})", ey = end_y, p = p),
        )
    } else {
        ("0".to_string(), "0".to_string())
    };

    (
        format!("({})+({})", in_expr.0, out_expr.0),
        format!("({})+({})", in_expr.1, out_expr.1),
    )
}

pub fn build_zoom_expr(
    duration: Duration,
    zoom_in_raw: f32,
    zoom_out_raw: f32,
    zoom_amount: f32,
    time_var: &str,
) -> String {
    let duration = duration.as_secs_f64();
    if duration <= 0.0 {
        return "1".to_string();
    }
    let zoom_in = (zoom_in_raw as f64).max(0.0).min(duration);
    let zoom_out = (zoom_out_raw as f64).max(0.0).min(duration);
    if zoom_in <= 0.001 && zoom_out <= 0.001 {
        return "1".to_string();
    }
    let amount = (zoom_amount as f64).clamp(0.1, 4.0);
    let mut expr = "1".to_string();
    if zoom_in > 0.001 {
        let p = format!("clip({t}/{d:.6},0,1)", t = time_var, d = zoom_in);
        let in_expr = format!("{a:.6} + (1-{a:.6})*({p})", a = amount, p = p);
        expr = format!(
            "if(lt({t},{d:.6}),{in_expr},{expr})",
            t = time_var,
            d = zoom_in,
            in_expr = in_expr,
            expr = expr
        );
    }
    if zoom_out > 0.001 {
        let start = (duration - zoom_out).max(0.0);
        let p = format!(
            "clip(({t}-{s:.6})/{d:.6},0,1)",
            t = time_var,
            s = start,
            d = zoom_out
        );
        let out_expr = format!("1 + ({a:.6}-1)*({p})", a = amount, p = p);
        expr = format!(
            "if(gt({t},{s:.6}),{out_expr},{expr})",
            t = time_var,
            s = start,
            out_expr = out_expr,
            expr = expr
        );
    }
    expr
}

pub fn build_shock_zoom_expr(
    duration: Duration,
    shock_in_raw: f32,
    shock_out_raw: f32,
    shock_amount: f32,
    time_var: &str,
) -> String {
    let duration = duration.as_secs_f64();
    if duration <= 0.0 {
        return "1".to_string();
    }
    let shock_in = (shock_in_raw as f64).max(0.0).min(duration);
    let shock_out = (shock_out_raw as f64).max(0.0).min(duration);
    if shock_in <= 0.001 && shock_out <= 0.001 {
        return "1".to_string();
    }
    let amount = (shock_amount as f64).clamp(0.1, 4.0);
    let decay = 6.0;
    let in_expr = if shock_in > 0.001 {
        format!(
            "if(lt({t},{d:.6}),exp(-{k:.3}*clip({t}/{d:.6},0,1)),0)",
            t = time_var,
            d = shock_in,
            k = decay
        )
    } else {
        "0".to_string()
    };
    let out_expr = if shock_out > 0.001 {
        let start = (duration - shock_out).max(0.0);
        format!(
            "if(gt({t},{s:.6}),exp(-{k:.3}*(1-clip(({t}-{s:.6})/{d:.6},0,1))),0)",
            t = time_var,
            s = start,
            d = shock_out,
            k = decay
        )
    } else {
        "0".to_string()
    };
    let pulse = format!(
        "if(gte({in_expr},{out_expr}),{in_expr},{out_expr})",
        in_expr = in_expr,
        out_expr = out_expr
    );
    format!("1 + ({a:.6}-1)*({p})", a = amount, p = pulse)
}

pub fn build_fade_expr(
    duration: Duration,
    fade_in_raw: f32,
    fade_out_raw: f32,
    time_var: &str,
) -> Option<String> {
    let duration = duration.as_secs_f64();
    if duration <= 0.0 {
        return None;
    }
    let fade_in = (fade_in_raw as f64).max(0.0).min(duration);
    let fade_out = (fade_out_raw as f64).max(0.0).min(duration);
    if fade_in <= 0.001 && fade_out <= 0.001 {
        return None;
    }

    let fade_in_expr = if fade_in > 0.001 {
        Some(format!(
            "if(lt({t},{fi:.6}),{t}/{fi:.6},1)",
            t = time_var,
            fi = fade_in
        ))
    } else {
        None
    };

    let fade_out_expr = if fade_out > 0.001 {
        let start = (duration - fade_out).max(0.0);
        Some(format!(
            "if(lt({t},{start:.6}),1,if(lt({t},{dur:.6}),({dur:.6}-{t})/{fo:.6},0))",
            t = time_var,
            start = start,
            dur = duration,
            fo = fade_out
        ))
    } else {
        None
    };

    let expr = match (fade_in_expr, fade_out_expr) {
        (Some(a), Some(b)) => format!("min({a},{b})"),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => return None,
    };

    Some(format!("clip({},0,1)", expr))
}

pub fn build_dissolve_expr(
    duration: Duration,
    dissolve_in_raw: f32,
    dissolve_out_raw: f32,
    time_var: &str,
) -> Option<String> {
    let duration = duration.as_secs_f64();
    if duration <= 0.0 {
        return None;
    }
    let dissolve_in = (dissolve_in_raw as f64).max(0.0).min(duration);
    let dissolve_out = (dissolve_out_raw as f64).max(0.0).min(duration);
    if dissolve_in <= 0.001 && dissolve_out <= 0.001 {
        return None;
    }

    let dissolve_in_expr = if dissolve_in > 0.001 {
        Some(format!(
            "if(lt({t},{di:.6}),{t}/{di:.6},1)",
            t = time_var,
            di = dissolve_in
        ))
    } else {
        None
    };

    let dissolve_out_expr = if dissolve_out > 0.001 {
        let start = (duration - dissolve_out).max(0.0);
        Some(format!(
            "if(lt({t},{start:.6}),1,if(lt({t},{dur:.6}),({dur:.6}-{t})/{do_:.6},0))",
            t = time_var,
            start = start,
            dur = duration,
            do_ = dissolve_out
        ))
    } else {
        None
    };

    let expr = match (dissolve_in_expr, dissolve_out_expr) {
        (Some(a), Some(b)) => format!("min({a},{b})"),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => return None,
    };

    Some(format!("clip({},0,1)", expr))
}
