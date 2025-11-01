#[derive(Debug, Clone)]
pub struct EVCalcResult {
    pub y_star: f64,
    pub ev: f64,
}

pub fn compute_ev_star_for_block(
    o: f64,
    t: f64,
    ore_value_in_sol: f64,
) -> EVCalcResult {
    const PROTOCOL_CUT: f64 = 0.10;
    const ADMIN_FEE: f64 = 0.01;
    const REF_MULT: f64 = 0.9;
    const P_WIN: f64 = 1.0 / 25.0;
    const ADMIN_COST_FACTOR: f64 = ADMIN_FEE / (1.0 - ADMIN_FEE);
    const C: f64 = 24.0 + ADMIN_COST_FACTOR / P_WIN;

    if o <= 0.0 || t <= 0.0 {
        return EVCalcResult { y_star: 0.0, ev: 0.0 };
    }

    let mut v = (1.0 - PROTOCOL_CUT) * (t - o) + ore_value_in_sol;
    if v <= 0.0 {
        return EVCalcResult { y_star: 0.0, ev: 0.0 };
    }

    let mut y_star = ((ore_value_in_sol * o) / C).sqrt();
    for _ in 0..3 {
        v = (1.0 - PROTOCOL_CUT) * (t - o - y_star) + ore_value_in_sol;
        if v <= 0.0 {
            break;
        }
        y_star = ((v * o) / C).sqrt() - o;
        if y_star < 0.0 {
            y_star = 0.0;
        }
    }

    let f = if o + y_star > 0.0 { y_star / (o + y_star) } else { 0.0 };
    let admin_cost = ADMIN_COST_FACTOR * y_star;
    let ev = P_WIN * (-24.0 * y_star + v * f) - admin_cost;

    EVCalcResult { y_star, ev }
}
