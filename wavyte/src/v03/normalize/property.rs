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

    // Layout inputs (animatable in v0.3; lane-typed)
    LayoutGapX = 15,
    LayoutGapY = 16,
    LayoutPaddingTopPx = 17,
    LayoutPaddingRightPx = 18,
    LayoutPaddingBottomPx = 19,
    LayoutPaddingLeftPx = 20,
    LayoutMarginTopPx = 21,
    LayoutMarginRightPx = 22,
    LayoutMarginBottomPx = 23,
    LayoutMarginLeftPx = 24,
    LayoutFlexGrow = 25,
    LayoutFlexShrink = 26,
    LayoutWidthPx = 27,
    LayoutHeightPx = 28,
    LayoutMinWidthPx = 29,
    LayoutMinHeightPx = 30,
    LayoutMaxWidthPx = 31,
    LayoutMaxHeightPx = 32,
}

impl PropertyKey {
    pub(crate) const COUNT: u32 = 33;

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
