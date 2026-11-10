//! Histórico do desfazer: cada passo guarda só o que mudou.
//!
//! Guardar a cena inteira a cada passo deixava, com o histórico cheio, 300
//! cópias do desenho na memória. Um [`Step`] guarda os elementos que eram
//! diferentes, os ids que não existiam e, só quando a ordem mudou, a ordem.
//! Tudo é achado pelo id, e não pela posição, para que uma mudança feita fora
//! do histórico (a quebra automática de um rótulo, por exemplo) não embaralhe
//! o passo: quem ele não cita fica como está.

use crate::model::Element;
use std::collections::{HashMap, HashSet};

/// Passos guardados; os mais antigos saem.
const MAX_STEPS: usize = 300;

/// Como chegar a uma cena (o alvo) a partir de outra (a base).
#[derive(Debug)]
enum Step {
    /// Ids repetidos em uma das cenas: guarda o alvo inteiro.
    Full(Vec<Element>),
    Delta {
        /// Elementos que as duas cenas têm, como são no alvo.
        changed: Vec<Element>,
        /// Elementos só do alvo, na ordem dele, cada um com o id de quem vem
        /// logo antes (`None` = é o primeiro).
        added: Vec<(Option<String>, Element)>,
        /// Ids só da base.
        gone: Vec<String>,
        /// A ordem do alvo, quando os elementos em comum trocaram de lugar.
        order: Option<Vec<String>>,
    },
}

fn unique_ids(els: &[Element]) -> bool {
    let mut seen = HashSet::with_capacity(els.len());
    els.iter().all(|e| seen.insert(e.id.as_str()))
}

/// Põe em `out` os elementos novos que vêm depois de `id`, e os que vêm
/// depois deles.
fn chain<'a>(
    out: &mut Vec<Element>,
    after: &mut HashMap<Option<&'a str>, Vec<&'a Element>>,
    placed: &mut HashSet<&'a str>,
    id: Option<&'a str>,
) {
    let mut key = id;
    while let Some(list) = after.remove(&key) {
        for e in &list {
            placed.insert(e.id.as_str());
            out.push((*e).clone());
        }
        key = list.last().map(|e| e.id.as_str());
    }
}

impl Step {
    /// O passo que leva de `base` a `target`; `None` se são iguais.
    fn between(base: &[Element], target: &[Element]) -> Option<Step> {
        if !unique_ids(base) || !unique_ids(target) {
            return (base != target).then(|| Step::Full(target.to_vec()));
        }
        let at: HashMap<&str, usize> = base
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id.as_str(), i))
            .collect();
        let in_target: HashSet<&str> = target.iter().map(|e| e.id.as_str()).collect();
        let gone: Vec<String> = base
            .iter()
            .filter(|e| !in_target.contains(e.id.as_str()))
            .map(|e| e.id.clone())
            .collect();
        let (mut changed, mut added) = (vec![], vec![]);
        let mut prev: Option<&str> = None;
        for e in target {
            match at.get(e.id.as_str()) {
                Some(&i) if base[i] == *e => {}
                Some(_) => changed.push(e.clone()),
                None => added.push((prev.map(str::to_string), e.clone())),
            }
            prev = Some(&e.id);
        }
        // os que ficaram continuam na mesma ordem entre si?
        let kept_base = base.iter().filter(|e| in_target.contains(e.id.as_str()));
        let kept_target = target.iter().filter(|e| at.contains_key(e.id.as_str()));
        let same_order = kept_base.map(|e| &e.id).eq(kept_target.map(|e| &e.id));
        let order = (!same_order).then(|| target.iter().map(|e| e.id.clone()).collect());
        if changed.is_empty() && added.is_empty() && gone.is_empty() && order.is_none() {
            return None;
        }
        Some(Step::Delta {
            changed,
            added,
            gone,
            order,
        })
    }

    /// O alvo, a partir da base.
    fn apply(&self, base: &[Element]) -> Vec<Element> {
        let (changed, added, gone, order) = match self {
            Step::Full(target) => return target.clone(),
            Step::Delta {
                changed,
                added,
                gone,
                order,
            } => (changed, added, gone, order),
        };
        let gone: HashSet<&str> = gone.iter().map(String::as_str).collect();
        let changed: HashMap<&str, &Element> = changed.iter().map(|e| (e.id.as_str(), e)).collect();
        let mut after: HashMap<Option<&str>, Vec<&Element>> = HashMap::new();
        for (anchor, e) in added {
            after.entry(anchor.as_deref()).or_default().push(e);
        }
        let mut out: Vec<Element> = Vec::with_capacity(base.len() + added.len());
        let mut placed: HashSet<&str> = HashSet::with_capacity(added.len());
        chain(&mut out, &mut after, &mut placed, None);
        for e in base {
            if gone.contains(e.id.as_str()) || placed.contains(e.id.as_str()) {
                continue;
            }
            out.push(changed.get(e.id.as_str()).copied().unwrap_or(e).clone());
            chain(&mut out, &mut after, &mut placed, Some(e.id.as_str()));
        }
        // a base não é mais a de quando o passo foi feito: o que sobrou vai
        // para o fim, em vez de sumir
        for (_, e) in added {
            if !placed.contains(e.id.as_str()) && !out.iter().any(|o| o.id == e.id) {
                out.push(e.clone());
            }
        }
        let Some(order) = order else { return out };
        let at: HashMap<String, usize> = out
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id.clone(), i))
            .collect();
        let mut slots: Vec<Option<Element>> = out.into_iter().map(Some).collect();
        let mut sorted: Vec<Element> = order
            .iter()
            .filter_map(|id| slots[*at.get(id)?].take())
            .collect();
        sorted.extend(slots.into_iter().flatten());
        sorted
    }
}

/// Os passos de desfazer e refazer de um desenho.
#[derive(Default)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
}

impl History {
    /// Registra que a cena foi de `before` para `now`; `false` se são iguais.
    pub fn push(&mut self, before: &[Element], now: &[Element]) -> bool {
        let Some(step) = Step::between(now, before) else {
            return false;
        };
        self.undo.push(step);
        if self.undo.len() > MAX_STEPS {
            self.undo.remove(0);
        }
        self.redo.clear();
        true
    }

    /// Volta um passo; `false` se não há o que desfazer.
    pub fn undo(&mut self, now: &mut Vec<Element>) -> bool {
        Self::travel(&mut self.undo, &mut self.redo, now)
    }

    /// Refaz o passo desfeito; `false` se não há o que refazer.
    pub fn redo(&mut self, now: &mut Vec<Element>) -> bool {
        Self::travel(&mut self.redo, &mut self.undo, now)
    }

    fn travel(from: &mut Vec<Step>, back: &mut Vec<Step>, now: &mut Vec<Element>) -> bool {
        let Some(step) = from.pop() else { return false };
        let there = step.apply(now);
        // um passo que não muda nada ainda conta, para as duas pilhas não
        // perderem o passo de volta
        back.push(Step::between(&there, now).unwrap_or_else(|| Step::Full(now.clone())));
        *now = there;
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(id: &str, x: f64) -> Element {
        let mut e = Element::new("rectangle", x, 0.0, &crate::model::Style::default());
        e.id = id.to_string();
        // sem os números sorteados, dois elementos feitos iguais são iguais
        (e.seed, e.version_nonce, e.updated) = (1, 1, 1);
        e
    }

    /// Números sempre iguais, para o teste repetir do mesmo jeito.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self, below: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as usize) % below.max(1)
        }
    }

    /// Uma ação qualquer do editor: mexer, criar, apagar, trocar de lugar.
    fn act(els: &mut Vec<Element>, rng: &mut Rng, next_id: &mut usize) {
        for _ in 0..1 + rng.next(3) {
            let n = els.len();
            match rng.next(6) {
                0 if n > 0 => els[rng.next(n)].x += 1.0 + rng.next(9) as f64,
                1 => {
                    // cola um grupo no meio ou no fim
                    let at = rng.next(n + 1);
                    for k in 0..1 + rng.next(3) {
                        *next_id += 1;
                        els.insert((at + k).min(els.len()), el(&format!("e{next_id}"), 0.0));
                    }
                }
                2 if n > 0 => {
                    els.remove(rng.next(n));
                }
                3 if n > 1 => {
                    let e = els.remove(rng.next(n));
                    els.insert(rng.next(n), e);
                }
                4 if n > 1 => els.swap(rng.next(n), rng.next(n)),
                _ => {
                    *next_id += 1;
                    els.push(el(&format!("e{next_id}"), 0.0));
                }
            }
        }
    }

    #[test]
    fn steps_rebuild_the_other_scene() {
        let a = vec![el("a", 0.0), el("b", 0.0), el("c", 0.0)];
        assert!(Step::between(&a, &a).is_none());
        // só mexeu: guarda um elemento, sem ordem
        let mut moved = a.clone();
        moved[1].x = 5.0;
        let Some(Step::Delta {
            changed,
            added,
            gone,
            order,
        }) = Step::between(&a, &moved)
        else {
            panic!("esperava um delta");
        };
        assert_eq!((changed.len(), added.len(), gone.len()), (1, 0, 0));
        assert!(order.is_none());
        // criar e apagar no meio também não precisa da ordem inteira
        let grown = vec![
            el("n0", 0.0),
            el("a", 0.0),
            el("n1", 0.0),
            el("n2", 0.0),
            el("c", 1.0),
            el("n3", 0.0),
        ];
        for (base, target) in [(&a, &grown), (&grown, &a)] {
            let step = Step::between(base, target).unwrap();
            assert!(matches!(&step, Step::Delta { order: None, .. }), "{step:?}");
            assert_eq!(&step.apply(base), target);
        }
        // trocar de lugar guarda a ordem
        let swapped = vec![el("c", 0.0), el("b", 0.0), el("a", 0.0)];
        let step = Step::between(&a, &swapped).unwrap();
        assert!(matches!(&step, Step::Delta { order: Some(_), .. }));
        assert_eq!(step.apply(&a), swapped);
        // ids repetidos: a cena inteira
        let twins = vec![el("a", 0.0), el("a", 1.0)];
        let step = Step::between(&a, &twins).unwrap();
        assert!(matches!(&step, Step::Full(_)));
        assert_eq!(step.apply(&a), twins);
        assert!(Step::between(&twins, &twins).is_none());
    }

    #[test]
    fn undo_and_redo_match_whole_copies_of_the_scene() {
        for seed in 0..40u64 {
            let mut rng = Rng(seed * 7919 + 1);
            let mut next_id = 0;
            let mut now: Vec<Element> = (0..rng.next(6))
                .map(|i| el(&format!("s{i}"), i as f64))
                .collect();
            let mut history = History::default();
            // o jeito antigo, para comparar: a cena inteira a cada passo
            let (mut undo, mut redo): (Vec<Vec<Element>>, Vec<Vec<Element>>) = (vec![], vec![]);
            for _ in 0..120 {
                match rng.next(10) {
                    0..=5 => {
                        let before = now.clone();
                        act(&mut now, &mut rng, &mut next_id);
                        if history.push(&before, &now) {
                            undo.push(before);
                            redo.clear();
                        } else {
                            assert_eq!(before, now, "semente {seed}");
                        }
                    }
                    6..=8 => {
                        let want = undo.pop();
                        assert_eq!(history.can_undo(), want.is_some());
                        if let Some(want) = want {
                            redo.push(now.clone());
                            assert!(history.undo(&mut now));
                            assert_eq!(now, want, "semente {seed}");
                        } else {
                            assert!(!history.undo(&mut now));
                        }
                    }
                    _ => {
                        let want = redo.pop();
                        assert_eq!(history.can_redo(), want.is_some());
                        if let Some(want) = want {
                            undo.push(now.clone());
                            assert!(history.redo(&mut now));
                            assert_eq!(now, want, "semente {seed}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_change_outside_the_history_does_not_scramble_a_step() {
        let mut now = vec![el("a", 0.0), el("b", 0.0)];
        let mut history = History::default();
        let before = now.clone();
        now[0].x = 10.0;
        now.push(el("c", 0.0));
        assert!(history.push(&before, &now));
        // fora do histórico: `b` muda, `c` some e aparece um `d`
        now[1].x = 3.0;
        now.pop();
        now.push(el("d", 0.0));
        assert!(history.undo(&mut now));
        // o passo desfaz o que ele fez e não toca no resto
        assert_eq!(now, vec![el("a", 0.0), el("b", 3.0), el("d", 0.0)]);
        // e o refazer volta para onde estava antes do desfazer
        assert!(history.redo(&mut now));
        assert_eq!(now, vec![el("a", 10.0), el("b", 3.0), el("d", 0.0)]);
    }

    #[test]
    fn old_steps_fall_off_and_a_new_one_clears_redo() {
        let mut now = vec![el("a", 0.0)];
        let mut history = History::default();
        for i in 0..MAX_STEPS + 20 {
            let before = now.clone();
            now[0].x = i as f64 + 1.0;
            history.push(&before, &now);
        }
        assert_eq!(history.undo.len(), MAX_STEPS);
        assert!(history.undo(&mut now) && history.can_redo());
        let before = now.clone();
        now[0].x = -1.0;
        assert!(history.push(&before, &now));
        assert!(!history.can_redo());
        history.clear();
        assert!(!history.can_undo() && !history.undo(&mut now));
    }
}
