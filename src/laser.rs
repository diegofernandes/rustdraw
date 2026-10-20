//! Ponteiro laser (K), como o do Excalidraw (`laser-trails.ts` e
//! `@excalidraw/laser-pointer`): um rastro vermelho que segue o ponteiro
//! enquanto o botão está apertado e some sozinho. Não cria elementos nem
//! entra no histórico.

use crate::geom::P;
use egui::{Color32, Pos2, Shape};

/// Tempo até um ponto do rastro sumir, em segundos.
const DECAY_TIME: f64 = 1.0;
/// Quantos pontos, a partir da ponta, o rastro mantém visíveis (afinando).
const DECAY_LENGTH: usize = 50;
/// Meia largura máxima do rastro, em pixels de tela (não muda com o zoom).
const SIZE: f32 = 2.0;
/// Suavização: cada ponto novo anda só 60% do caminho até o ponteiro.
const STREAMLINE: f32 = 0.4;
/// `DEFAULT_LASER_COLOR` do Excalidraw ("red").
pub const COLOR: Color32 = Color32::from_rgb(0xff, 0x00, 0x00);

struct Trail {
    /// Pontos já suavizados, em coordenadas do desenho, com o instante em que entraram.
    points: Vec<(P, f64)>,
    /// Último ponto bruto recebido (pontos repetidos são ignorados).
    last_raw: P,
}

#[derive(Default)]
pub struct Laser {
    trails: Vec<Trail>,
    drawing: bool,
}

fn ease_out(k: f32) -> f32 {
    1.0 - (1.0 - k).powi(4)
}

/// Meia largura do ponto `i` de `len`, que entrou no instante `t`.
fn size(i: usize, len: usize, t: f64, now: f64) -> f32 {
    let age = (1.0 - (now - t) / DECAY_TIME).max(0.0) as f32;
    let from_head = (len - i).min(DECAY_LENGTH);
    let length = (DECAY_LENGTH - from_head) as f32 / DECAY_LENGTH as f32;
    SIZE * ease_out(length).min(ease_out(age))
}

impl Laser {
    pub fn start(&mut self, p: P, now: f64) {
        self.trails.push(Trail {
            points: vec![(p, now)],
            last_raw: p,
        });
        self.drawing = true;
    }

    pub fn add(&mut self, p: P, now: f64) {
        if !self.drawing {
            return;
        }
        let Some(trail) = self.trails.last_mut() else {
            return;
        };
        if trail.last_raw == p {
            return;
        }
        trail.last_raw = p;
        let last = trail.points.last().map_or(p, |q| q.0);
        trail
            .points
            .push((last + (p - last) * (1.0 - STREAMLINE), now));
    }

    pub fn end(&mut self) {
        self.drawing = false;
    }

    /// Há rastro na tela (ou sendo desenhado): o quadro precisa ser redesenhado.
    pub fn active(&self) -> bool {
        self.drawing || !self.trails.is_empty()
    }

    /// Descarta pontos que já sumiram e rastros vazios.
    pub fn prune(&mut self, now: f64) {
        let last = self.trails.len().saturating_sub(1);
        let drawing = self.drawing;
        let mut i = 0;
        self.trails.retain_mut(|t| {
            let current = drawing && i == last;
            i += 1;
            // o tempo e a distância até a ponta não mudam ao tirar pontos do começo
            let len = t.points.len();
            let gone = t
                .points
                .iter()
                .enumerate()
                .take_while(|(j, (_, at))| len - j > DECAY_LENGTH || now - at >= DECAY_TIME)
                .count();
            // o rastro em andamento guarda a ponta, base da suavização
            let gone = if current { gone.min(len - 1) } else { gone };
            t.points.drain(..gone);
            current || !t.points.is_empty()
        });
    }

    /// Formas do rastro em coordenadas de tela.
    pub fn shapes(&self, now: f64, to_screen: impl Fn(P) -> Pos2) -> Vec<Shape> {
        let mut out = vec![];
        for t in &self.trails {
            let len = t.points.len();
            if len < 2 {
                continue;
            }
            let pts: Vec<(Pos2, f32)> = t
                .points
                .iter()
                .enumerate()
                .map(|(i, (p, at))| (to_screen(*p), size(i, len, *at, now)))
                .collect();
            for w in pts.windows(2) {
                let ((a, sa), (b, sb)) = (w[0], w[1]);
                if sa.max(sb) < 0.1 {
                    continue;
                }
                let d = b - a;
                if d.length() > 0.01 {
                    let n = d.normalized().rot90();
                    out.push(Shape::convex_polygon(
                        vec![a + n * sa, b + n * sb, b - n * sb, a - n * sa],
                        COLOR,
                        egui::Stroke::NONE,
                    ));
                }
            }
            // juntas e pontas arredondadas
            for (p, s) in pts {
                if s >= 0.1 {
                    out.push(Shape::circle_filled(p, s, COLOR));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(laser: &mut Laser, n: usize, t0: f64) {
        laser.start(P::new(0.0, 0.0), t0);
        for i in 1..n {
            laser.add(P::new(i as f32 * 10.0, 0.0), t0 + i as f64 * 0.01);
        }
    }

    #[test]
    fn trail_fades_out_by_itself() {
        let mut laser = Laser::default();
        draw(&mut laser, 20, 0.0);
        laser.end();
        let now = 0.2;
        laser.prune(now);
        assert!(laser.active());
        assert!(!laser.shapes(now, |p| Pos2::new(p.x, p.y)).is_empty());
        // um segundo depois do último ponto, nada sobra
        let later = 0.2 + DECAY_TIME;
        laser.prune(later);
        assert!(!laser.active());
        assert!(laser.shapes(later, |p| Pos2::new(p.x, p.y)).is_empty());
    }

    #[test]
    fn only_the_tail_near_the_head_stays() {
        let mut laser = Laser::default();
        draw(&mut laser, 200, 0.0);
        laser.prune(0.0);
        assert_eq!(laser.trails[0].points.len(), DECAY_LENGTH);
        // a ponta é a parte mais grossa
        let len = DECAY_LENGTH;
        assert!(size(len - 1, len, 2.0, 2.0) > size(len / 2, len, 2.0, 2.0));
        assert_eq!(size(0, len, 2.0, 2.0), 0.0);
    }

    #[test]
    fn stopped_pointer_keeps_head_while_drawing() {
        let mut laser = Laser::default();
        draw(&mut laser, 5, 0.0);
        laser.prune(10.0);
        assert_eq!(laser.trails[0].points.len(), 1);
        // repetir o mesmo ponto não cria pontos novos
        laser.add(P::new(40.0, 0.0), 10.0);
        assert_eq!(laser.trails[0].points.len(), 1);
        laser.add(P::new(50.0, 0.0), 10.0);
        let pts = &laser.trails[0].points;
        assert_eq!(pts.len(), 2);
        // suavizado: anda 60% do caminho até o ponteiro
        assert!(pts[1].0.x > pts[0].0.x && pts[1].0.x < 50.0);
    }
}
