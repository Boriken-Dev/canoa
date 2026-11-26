//! The ribbon: one tab with one button, for the windows of each application that has a ribbon. The labels carry the brand's name,
//! which cannot hold a quote or a markup character.

/// The callback the button names; `AddIn`'s `IDispatch` answers it.
pub(crate) const TOGGLE_CALLBACK: &str = "OnTogglePane";

const XML: &str = concat!(
    r#"<customUI xmlns="http://schemas.microsoft.com/office/2009/07/customui"><ribbon><tabs>"#,
    r#"<tab id="chatTab" label=""#,
    env!("CANOA_BRAND_NAME"),
    r#""><group id="chatGroup" label=""#,
    env!("CANOA_BRAND_NAME"),
    r#"">"#,
    r#"<button id="chatPane" label=""#,
    env!("CANOA_BRAND_NAME"),
    r#" chat" size="large" getImage="GetImage" "#,
    r#"screentip="Show or hide the "#,
    env!("CANOA_BRAND_NAME"),
    r#" pane" onAction="OnTogglePane"/>"#,
    r#"</group></tab></tabs></ribbon></customUI>"#
);

/// The callback that returns the button's picture.
pub(crate) const IMAGE_CALLBACK: &str = "GetImage";

/// The ribbons Office asks the add-in for, by the id it asks with.
const RIBBON_IDS: &[&str] = &[
    "Microsoft.Word.Document",
    "Microsoft.Excel.Workbook",
    "Microsoft.PowerPoint.Presentation",
    "Microsoft.Outlook.Explorer",
    "Microsoft.Outlook.Mail.Read",
    "Microsoft.Outlook.Mail.Compose",
];

/// The ribbon XML for the ribbon `id` names, or `None` for one the add-in adds nothing to.
pub fn ribbon_xml(id: &str) -> Option<&'static str> {
    RIBBON_IDS.contains(&id).then_some(XML)
}
