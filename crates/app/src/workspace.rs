//! Modelo de pestañas y paneles divididos (sin AppKit ni Metal).
//!
//! Cada pestaña tiene un árbol binario: las hojas son paneles y los nodos
//! internos, divisiones con una proporción. Los rectángulos se calculan aquí
//! para la vista (ratón) y para el render (dónde pintar cada panel).

pub type PaneId = u64;

/// Rectángulo en píxeles (origen arriba a la izquierda).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// Cómo se reparte el espacio en una división.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Lado a lado (Cmd+D): el divisor es vertical.
    Columns,
    /// Uno encima de otro (Cmd+Shift+D): el divisor es horizontal.
    Rows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq)]
enum Node {
    Leaf(PaneId),
    Split {
        axis: Axis,
        /// Fracción del espacio para `a`, en `[0.1, 0.9]`.
        ratio: f64,
        a: Box<Node>,
        b: Box<Node>,
    },
}

/// Un divisor: su rectángulo (zona visible de `thickness` píxeles) y el
/// camino hasta su división en el árbol, para arrastrarlo.
#[derive(Debug, Clone, PartialEq)]
pub struct Divider {
    pub rect: Rect,
    pub axis: Axis,
    /// Rectángulo de la división completa, para convertir el ratón en proporción.
    pub parent: Rect,
    path: Vec<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    root: Node,
    pub focused: PaneId,
}

impl Tab {
    pub fn new(pane: PaneId) -> Tab {
        Tab {
            root: Node::Leaf(pane),
            focused: pane,
        }
    }

    pub fn panes(&self) -> Vec<PaneId> {
        fn walk(n: &Node, out: &mut Vec<PaneId>) {
            match n {
                Node::Leaf(id) => out.push(*id),
                Node::Split { a, b, .. } => {
                    walk(a, out);
                    walk(b, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }

    /// Divide `target` y pone `new` a su derecha o debajo, con el foco.
    pub fn split(&mut self, target: PaneId, axis: Axis, new: PaneId) {
        fn walk(n: &mut Node, target: PaneId, axis: Axis, new: PaneId) -> bool {
            match n {
                Node::Leaf(id) if *id == target => {
                    *n = Node::Split {
                        axis,
                        ratio: 0.5,
                        a: Box::new(Node::Leaf(target)),
                        b: Box::new(Node::Leaf(new)),
                    };
                    true
                }
                Node::Leaf(_) => false,
                Node::Split { a, b, .. } => {
                    walk(a, target, axis, new) || walk(b, target, axis, new)
                }
            }
        }
        if walk(&mut self.root, target, axis, new) {
            self.focused = new;
        }
    }

    /// Quita un panel; su hermano ocupa su sitio. Devuelve `false` si la
    /// pestaña se queda sin paneles.
    pub fn close(&mut self, pane: PaneId) -> bool {
        fn walk(n: &mut Node, pane: PaneId) -> bool {
            let Node::Split { a, b, .. } = n else {
                return false;
            };
            let keep = if **a == Node::Leaf(pane) {
                Some(std::mem::replace(&mut **b, Node::Leaf(0)))
            } else if **b == Node::Leaf(pane) {
                Some(std::mem::replace(&mut **a, Node::Leaf(0)))
            } else {
                None
            };
            match keep {
                Some(sibling) => {
                    *n = sibling;
                    true
                }
                None => walk(a, pane) || walk(b, pane),
            }
        }
        if self.root == Node::Leaf(pane) {
            return false;
        }
        walk(&mut self.root, pane);
        if self.focused == pane {
            self.focused = self.panes()[0];
        }
        true
    }

    /// Rectángulo de cada panel y los divisores, dentro de `area`.
    pub fn layout(&self, area: Rect, thickness: f64) -> (Vec<(PaneId, Rect)>, Vec<Divider>) {
        fn walk(
            n: &Node,
            r: Rect,
            t: f64,
            path: &mut Vec<bool>,
            panes: &mut Vec<(PaneId, Rect)>,
            dividers: &mut Vec<Divider>,
        ) {
            match n {
                Node::Leaf(id) => panes.push((*id, r)),
                Node::Split { axis, ratio, a, b } => {
                    let (ra, rd, rb) = match axis {
                        Axis::Columns => {
                            let wa = ((r.w - t) * ratio).round();
                            (
                                Rect { w: wa, ..r },
                                Rect {
                                    x: r.x + wa,
                                    w: t,
                                    ..r
                                },
                                Rect {
                                    x: r.x + wa + t,
                                    w: r.w - wa - t,
                                    ..r
                                },
                            )
                        }
                        Axis::Rows => {
                            let ha = ((r.h - t) * ratio).round();
                            (
                                Rect { h: ha, ..r },
                                Rect {
                                    y: r.y + ha,
                                    h: t,
                                    ..r
                                },
                                Rect {
                                    y: r.y + ha + t,
                                    h: r.h - ha - t,
                                    ..r
                                },
                            )
                        }
                    };
                    dividers.push(Divider {
                        rect: rd,
                        axis: *axis,
                        parent: r,
                        path: path.clone(),
                    });
                    path.push(false);
                    walk(a, ra, t, path, panes, dividers);
                    path.pop();
                    path.push(true);
                    walk(b, rb, t, path, panes, dividers);
                    path.pop();
                }
            }
        }
        let (mut panes, mut dividers) = (Vec::new(), Vec::new());
        walk(
            &self.root,
            area,
            thickness,
            &mut Vec::new(),
            &mut panes,
            &mut dividers,
        );
        (panes, dividers)
    }

    /// Mueve el divisor para que `a` ocupe `pos` (coordenada del ratón en
    /// el eje de la división) dentro del rectángulo de la división.
    pub fn drag_divider(&mut self, divider: &Divider, pos: f64) {
        let mut n = &mut self.root;
        for &right in &divider.path {
            let Node::Split { a, b, .. } = n else { return };
            n = if right { b } else { a };
        }
        if let Node::Split { ratio, axis, .. } = n {
            let p = divider.parent;
            // El reparto es del espacio sin el divisor (ver `layout`).
            let r = match axis {
                Axis::Columns => (pos - p.x) / (p.w - divider.rect.w),
                Axis::Rows => (pos - p.y) / (p.h - divider.rect.h),
            };
            *ratio = r.clamp(0.1, 0.9);
        }
    }

    /// Panel vecino del que tiene el foco en una dirección, si lo hay.
    pub fn neighbor(&self, dir: Direction) -> Option<PaneId> {
        let area = Rect {
            x: 0.0,
            y: 0.0,
            w: 10_000.0,
            h: 10_000.0,
        };
        let (panes, _) = self.layout(area, 1.0);
        let cur = panes.iter().find(|(id, _)| *id == self.focused)?.1;
        let (cx, cy) = cur.center();
        panes
            .iter()
            .filter(|(id, _)| *id != self.focused)
            .filter(|(_, r)| match dir {
                // Al lado y solapando en el otro eje.
                Direction::Left => r.x + r.w <= cur.x + 1.0 && overlaps(r.y, r.h, cur.y, cur.h),
                Direction::Right => r.x >= cur.x + cur.w - 1.0 && overlaps(r.y, r.h, cur.y, cur.h),
                Direction::Up => r.y + r.h <= cur.y + 1.0 && overlaps(r.x, r.w, cur.x, cur.w),
                Direction::Down => r.y >= cur.y + cur.h - 1.0 && overlaps(r.x, r.w, cur.x, cur.w),
            })
            .min_by(|(_, a), (_, b)| {
                let d = |r: &Rect| {
                    let (x, y) = r.center();
                    (x - cx).abs() + (y - cy).abs()
                };
                d(a).total_cmp(&d(b))
            })
            .map(|(id, _)| *id)
    }
}

fn overlaps(a: f64, alen: f64, b: f64, blen: f64) -> bool {
    a < b + blen && b < a + alen
}

/// Las pestañas de una ventana.
#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    pub tabs: Vec<Tab>,
    pub active: usize,
}

impl Workspace {
    pub fn new(first: PaneId) -> Workspace {
        Workspace {
            tabs: vec![Tab::new(first)],
            active: 0,
        }
    }

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    pub fn focused(&self) -> PaneId {
        self.tab().focused
    }

    /// Añade una pestaña detrás de la activa y la activa.
    pub fn new_tab(&mut self, pane: PaneId) {
        self.active += 1;
        self.tabs.insert(self.active, Tab::new(pane));
    }

    /// Quita una pestaña. Devuelve `false` si no queda ninguna.
    pub fn close_tab(&mut self, index: usize) -> bool {
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            return false;
        }
        if self.active > index || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        true
    }

    /// Cierra un panel esté en la pestaña que esté. Devuelve `false` si no
    /// queda ninguna pestaña.
    pub fn close_pane(&mut self, pane: PaneId) -> bool {
        let Some(index) = self.tabs.iter().position(|t| t.panes().contains(&pane)) else {
            return true;
        };
        if self.tabs[index].close(pane) {
            true
        } else {
            self.close_tab(index)
        }
    }

    pub fn select(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
        }
    }

    pub fn next(&mut self) {
        self.active = (self.active + 1) % self.tabs.len();
    }

    pub fn previous(&mut self) {
        self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
    }

    #[cfg(test)]
    pub fn all_panes(&self) -> Vec<PaneId> {
        self.tabs.iter().flat_map(Tab::panes).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 101.0,
        h: 51.0,
    };

    #[test]
    fn split_gives_focus_and_lays_out_side_by_side() {
        let mut t = Tab::new(1);
        t.split(1, Axis::Columns, 2);
        assert_eq!(t.focused, 2);
        let (panes, dividers) = t.layout(AREA, 1.0);
        assert_eq!(
            panes[0],
            (
                1,
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 50.0,
                    h: 51.0
                }
            )
        );
        assert_eq!(
            panes[1],
            (
                2,
                Rect {
                    x: 51.0,
                    y: 0.0,
                    w: 50.0,
                    h: 51.0
                }
            )
        );
        assert_eq!(
            dividers[0].rect,
            Rect {
                x: 50.0,
                y: 0.0,
                w: 1.0,
                h: 51.0
            }
        );
    }

    #[test]
    fn nested_splits_and_close() {
        let mut t = Tab::new(1);
        t.split(1, Axis::Columns, 2);
        t.split(2, Axis::Rows, 3);
        assert_eq!(t.panes(), [1, 2, 3]);
        let (panes, _) = t.layout(AREA, 1.0);
        assert_eq!(
            panes[1].1,
            Rect {
                x: 51.0,
                y: 0.0,
                w: 50.0,
                h: 25.0
            }
        );
        assert_eq!(
            panes[2].1,
            Rect {
                x: 51.0,
                y: 26.0,
                w: 50.0,
                h: 25.0
            }
        );
        // Al cerrar 2, el 3 ocupa toda la columna derecha.
        assert!(t.close(2));
        let (panes, _) = t.layout(AREA, 1.0);
        assert_eq!(
            panes[1],
            (
                3,
                Rect {
                    x: 51.0,
                    y: 0.0,
                    w: 50.0,
                    h: 51.0
                }
            )
        );
        assert_eq!(t.focused, 3);
        assert!(t.close(1));
        assert!(!t.close(3));
    }

    #[test]
    fn focus_moves_by_direction() {
        // [1 | 2]
        //     [-]
        //     [3]
        let mut t = Tab::new(1);
        t.split(1, Axis::Columns, 2);
        t.split(2, Axis::Rows, 3);
        assert_eq!(t.neighbor(Direction::Up), Some(2));
        assert_eq!(t.neighbor(Direction::Left), Some(1));
        t.focused = 1;
        assert_eq!(
            t.neighbor(Direction::Right).map(|p| p == 2 || p == 3),
            Some(true)
        );
        assert_eq!(t.neighbor(Direction::Left), None);
    }

    #[test]
    fn dragging_a_divider_changes_the_ratio() {
        let mut t = Tab::new(1);
        t.split(1, Axis::Columns, 2);
        let (_, dividers) = t.layout(AREA, 1.0);
        t.drag_divider(&dividers[0], 75.0);
        let (panes, _) = t.layout(AREA, 1.0);
        assert_eq!(panes[0].1.w, 75.0);
        // Con límites: nunca menos del 10 %.
        t.drag_divider(&dividers[0], 0.0);
        let (panes, _) = t.layout(AREA, 1.0);
        assert_eq!(panes[0].1.w, 10.0);
    }

    #[test]
    fn tabs_open_close_and_cycle() {
        let mut w = Workspace::new(1);
        w.new_tab(2);
        w.new_tab(3);
        assert_eq!(w.active, 2);
        w.select(0);
        w.new_tab(4);
        // La nueva va justo detrás de la activa.
        assert_eq!(
            w.tabs.iter().map(|t| t.focused).collect::<Vec<_>>(),
            [1, 4, 2, 3]
        );
        w.previous();
        assert_eq!(w.focused(), 1);
        w.previous();
        assert_eq!(w.focused(), 3);
        w.next();
        assert_eq!(w.focused(), 1);
        assert!(w.close_tab(0));
        assert_eq!(w.focused(), 4);
        assert!(w.close_pane(4) && w.close_pane(2));
        assert!(!w.close_pane(3));
    }

    #[test]
    fn closing_last_pane_of_a_tab_closes_the_tab() {
        let mut w = Workspace::new(1);
        w.new_tab(2);
        w.tab_mut().split(2, Axis::Rows, 3);
        assert!(w.close_pane(2));
        assert_eq!(w.tabs.len(), 2);
        assert!(w.close_pane(3));
        assert_eq!(w.tabs.len(), 1);
        assert_eq!(w.all_panes(), [1]);
    }
}
