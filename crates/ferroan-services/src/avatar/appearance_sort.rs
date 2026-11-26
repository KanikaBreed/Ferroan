// Licensed to the .NET Foundation under one or more agreements.
// The .NET Foundation licenses this file to you under the MIT license.
//
// The MIT License (MIT)
//
// Copyright (c) .NET Foundation and Contributors
//
// All rights reserved.
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.
//
// Ported from src/runtime/src/libraries/System.Private.CoreLib/src/System/Collections/Generic/ArraySortHelper.cs
//   dotnet/dotnet @ 0427b9abbead3fa64c6655eb4290c537397131fa (.NET 10.0.14).
// Ferroan changes: Copyright (c) 2026 Ferroan contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use super::AppearanceItem;

type Row = (usize, i32, AppearanceItem);

fn key(row: &Row) -> (usize, i32) {
    (row.0, row.1)
}

/// Pin8 uses List.Sort(Comparison<T>). Its ordering of equal (type, index)
/// keys determines the order in which the viewer draws those layers.
pub(super) fn sort(rows: &mut [Row]) {
    if rows.len() > 1 {
        intro_sort(rows, 2 * (rows.len().ilog2() + 1));
    }
}

fn swap_if_greater(rows: &mut [Row], left: usize, right: usize) {
    if key(&rows[left]) > key(&rows[right]) {
        rows.swap(left, right);
    }
}

fn intro_sort(mut rows: &mut [Row], mut depth: u32) {
    while rows.len() > 1 {
        match rows.len() {
            2 => {
                swap_if_greater(rows, 0, 1);
                return;
            }
            3 => {
                swap_if_greater(rows, 0, 1);
                swap_if_greater(rows, 0, 2);
                swap_if_greater(rows, 1, 2);
                return;
            }
            4..=16 => {
                insertion_sort(rows);
                return;
            }
            _ if depth == 0 => {
                heap_sort(rows);
                return;
            }
            _ => {
                depth -= 1;
                let pivot = partition(rows);
                intro_sort(&mut rows[pivot + 1..], depth);
                rows = &mut rows[..pivot];
            }
        }
    }
}

fn insertion_sort(rows: &mut [Row]) {
    for i in 1..rows.len() {
        let row = rows[i];
        let mut j = i;
        while j > 0 && key(&row) < key(&rows[j - 1]) {
            rows[j] = rows[j - 1];
            j -= 1;
        }
        rows[j] = row;
    }
}

fn partition(rows: &mut [Row]) -> usize {
    let high = rows.len() - 1;
    let middle = high / 2;
    swap_if_greater(rows, 0, middle);
    swap_if_greater(rows, 0, high);
    swap_if_greater(rows, middle, high);

    let pivot = key(&rows[middle]);
    rows.swap(middle, high - 1);
    let mut left = 0;
    let mut right = high - 1;
    // The sorted endpoints and parked pivot terminate both scans.
    loop {
        left += 1;
        while key(&rows[left]) < pivot {
            left += 1;
        }
        right -= 1;
        while pivot < key(&rows[right]) {
            right -= 1;
        }
        if left >= right {
            break;
        }
        rows.swap(left, right);
    }
    rows.swap(left, high - 1);
    left
}

fn heap_sort(rows: &mut [Row]) {
    let len = rows.len();
    for root in (1..=len / 2).rev() {
        down_heap(rows, root);
    }
    for end in (1..len).rev() {
        rows.swap(0, end);
        down_heap(&mut rows[..end], 1);
    }
}

// The heap uses one-based positions, as in ArraySortHelper.DownHeap.
fn down_heap(rows: &mut [Row], mut root: usize) {
    let row = rows[root - 1];
    while root <= rows.len() / 2 {
        let mut child = 2 * root;
        if child < rows.len() && key(&rows[child - 1]) < key(&rows[child]) {
            child += 1;
        }
        if key(&row) >= key(&rows[child - 1]) {
            break;
        }
        rows[root - 1] = rows[child - 1];
        root = child;
    }
    rows[root - 1] = row;
}
