use crate::v03::foundation::ids::{NodeIdx, PropertyId};

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum PropertyKey {
    Opacity = 0,
    TransformTranslateX = 1,
    TransformTranslateY = 2,
    TransformRotationRad = 3,
    TransformScaleX = 4,
    TransformScaleY = 5,
    TransformAnchorX = 6,
    TransformAnchorY = 7,
    TransformSkewX = 8,
    TransformSkewY = 9,
    SwitchActiveIndex = 10,

    // Reserved (planned-but-disabled in v0.3)
    LayoutX = 11,
    LayoutY = 12,
    LayoutWidth = 13,
    LayoutHeight = 14,
}

impl PropertyKey {
    pub(crate) const COUNT: u32 = 15;

    pub(crate) fn as_u32(self) -> u32 {
        self as u16 as u32
    }
}

pub(crate) struct PropertyIndex;

impl PropertyIndex {
    pub(crate) fn property_id(node: NodeIdx, key: PropertyKey) -> PropertyId {
        let node = node.0;
        let key = key.as_u32();
        PropertyId(node.saturating_mul(PropertyKey::COUNT).saturating_add(key))
    }
}
