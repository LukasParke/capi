package main

import "embed"

//go:embed templates/*.gohtml
var uiTemplatesFS embed.FS

//go:embed static/*
var uiStaticFS embed.FS
