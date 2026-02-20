package tui

import "strings"

func UnicodeTable(headers []string, rows [][]string) string {
	cols := len(headers)
	for _, row := range rows {
		if len(row) > cols {
			cols = len(row)
		}
	}
	if cols == 0 {
		return ""
	}

	widths := make([]int, cols)
	for i := 0; i < cols; i++ {
		if i < len(headers) {
			widths[i] = visibleWidth(headers[i])
		}
		for _, row := range rows {
			if i < len(row) {
				if w := visibleWidth(row[i]); w > widths[i] {
					widths[i] = w
				}
			}
		}
		if widths[i] < 1 {
			widths[i] = 1
		}
	}

	var b strings.Builder
	b.WriteString(tableBorder("┌", "┬", "┐", widths))
	b.WriteString("\n")
	if len(headers) > 0 {
		b.WriteString(tableRow(headers, widths))
		b.WriteString("\n")
		b.WriteString(tableBorder("├", "┼", "┤", widths))
	}
	for i, row := range rows {
		b.WriteString("\n")
		b.WriteString(tableRow(row, widths))
		if i < len(rows)-1 {
			b.WriteString("\n")
			b.WriteString(tableBorder("├", "┼", "┤", widths))
		}
	}
	b.WriteString("\n")
	b.WriteString(tableBorder("└", "┴", "┘", widths))
	return b.String()
}

func tableBorder(left, join, right string, widths []int) string {
	var b strings.Builder
	b.WriteString(left)
	for i, w := range widths {
		b.WriteString(strings.Repeat("─", w+2))
		if i < len(widths)-1 {
			b.WriteString(join)
		}
	}
	b.WriteString(right)
	return b.String()
}

func tableRow(cells []string, widths []int) string {
	var b strings.Builder
	b.WriteString("│")
	for i, w := range widths {
		cell := ""
		if i < len(cells) {
			cell = cells[i]
		}
		pad := w - visibleWidth(cell)
		if pad < 0 {
			pad = 0
		}
		b.WriteString(" ")
		b.WriteString(cell)
		b.WriteString(strings.Repeat(" ", pad))
		b.WriteString(" │")
	}
	return b.String()
}
