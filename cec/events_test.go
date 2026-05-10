package cec

import (
	"sync"
	"testing"
)

func TestEventStatsCounters(t *testing.T) {
	var s EventStats
	if s.Delivered() != 0 || s.Dropped() != 0 {
		t.Fatalf("zero stats expected, got delivered=%d dropped=%d", s.Delivered(), s.Dropped())
	}
	s.delivered.Add(3)
	s.dropped.Add(2)
	if s.Delivered() != 3 || s.Dropped() != 2 {
		t.Fatalf("expected 3/2, got %d/%d", s.Delivered(), s.Dropped())
	}
}

// TestEventStatsRace exercises the atomic counters under concurrent writers
// to verify there is no data race (run with -race).
func TestEventStatsRace(t *testing.T) {
	var s EventStats
	const N = 1000
	var wg sync.WaitGroup
	for i := 0; i < 4; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := 0; j < N; j++ {
				s.delivered.Add(1)
				if j%3 == 0 {
					s.dropped.Add(1)
				}
			}
		}()
	}
	wg.Wait()
	if got := s.Delivered(); got != 4*N {
		t.Errorf("delivered = %d, want %d", got, 4*N)
	}
}

// TestDispatchNonBlockingDrops verifies that dispatch drops events when the
// channel is full instead of blocking, and that the counters reflect both
// successful sends and drops.
func TestDispatchNonBlockingDrops(t *testing.T) {
	c := &Connection{events: make(chan Event, 2)}
	for i := 0; i < 5; i++ {
		c.dispatch(Event{Kind: EventLog})
	}
	if got := c.stats.Delivered(); got != 2 {
		t.Errorf("delivered = %d, want 2", got)
	}
	if got := c.stats.Dropped(); got != 3 {
		t.Errorf("dropped = %d, want 3", got)
	}
}

// TestDispatchConcurrent posts events from multiple goroutines while a
// consumer drains the channel. Verifies no data races (run with -race) and
// that delivered+dropped == total posted.
func TestDispatchConcurrent(t *testing.T) {
	c := &Connection{events: make(chan Event, 4)}
	const writers = 4
	const perWriter = 250
	const total = writers * perWriter

	doneCh := make(chan struct{})
	go func() {
		for range c.events {
		}
		close(doneCh)
	}()

	var wg sync.WaitGroup
	for i := 0; i < writers; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := 0; j < perWriter; j++ {
				c.dispatch(Event{Kind: EventCommand})
			}
		}()
	}
	wg.Wait()
	close(c.events)
	<-doneCh

	if got := c.stats.Delivered() + c.stats.Dropped(); got != total {
		t.Errorf("delivered+dropped = %d, want %d", got, total)
	}
}
