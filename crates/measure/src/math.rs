pub(super) const fn lcm(a: u32, b: u32) -> u32 { (a / gcd(a, b)) * b }

pub(super) const fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) struct Frac {
    pub(super) num: u32,
    pub(super) den: u32,
}

pub(super) const fn reduce(f: Frac) -> Frac {
    let g = gcd(f.num, f.den);
    Frac { num: f.num / g, den: f.den / g }
}

