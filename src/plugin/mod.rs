// LanguagePlugin trait and registry — defined in Plan 02
// HARD BOUNDARY: NO TOKIO IMPORTS IN THIS DIRECTORY
// plugins/extract() is synchronous and runs on rayon threads only
// See PITFALLS.md Pitfall 4: rayon/tokio deadlock
