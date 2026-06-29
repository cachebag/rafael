import { useEffect, useMemo, useState } from "react";
import {
  CalendarDays,
  Check,
  ChevronDown,
  Dumbbell,
  LineChart,
  Moon,
  Plus,
  Save,
  Sun,
  Trash2
} from "lucide-react";
import {
  CartesianGrid,
  Line,
  LineChart as ReLineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis
} from "recharts";
import { loadState, saveState } from "./api";
import { DayKey, EntrySet, Exercise, JournalEntry, LiftState, Workout } from "./types";

type View = "journal" | "plan" | "progress";

interface SelectOption {
  value: string;
  label: string;
}

const dayKeys: DayKey[] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const dayLabels: Record<DayKey, string> = {
  mon: "Mon",
  tue: "Tue",
  wed: "Wed",
  thu: "Thu",
  fri: "Fri",
  sat: "Sat",
  sun: "Sun"
};

function id() {
  return crypto.randomUUID?.() ?? Math.random().toString(36).slice(2);
}

function defaultState(): LiftState {
  const legs = id();
  const push = id();
  const pull = id();

  return {
    version: 1,
    workouts: [
      {
        id: legs,
        name: "Legs",
        exercises: [
          { id: id(), name: "Squat", sets: 3, reps: 5 },
          { id: id(), name: "Romanian deadlift", sets: 3, reps: 8 },
          { id: id(), name: "Leg press", sets: 3, reps: 10 }
        ]
      },
      {
        id: push,
        name: "Upper push",
        exercises: [
          { id: id(), name: "Bench press", sets: 3, reps: 5 },
          { id: id(), name: "Overhead press", sets: 3, reps: 6 },
          { id: id(), name: "Incline dumbbell press", sets: 3, reps: 8 }
        ]
      },
      {
        id: pull,
        name: "Upper pull",
        exercises: [
          { id: id(), name: "Deadlift", sets: 3, reps: 5 },
          { id: id(), name: "Pull-up", sets: 3, reps: 8 },
          { id: id(), name: "Row", sets: 3, reps: 8 }
        ]
      }
    ],
    schedule: {
      mon: legs,
      tue: null,
      wed: push,
      thu: null,
      fri: pull,
      sat: null,
      sun: null
    },
    entries: {}
  };
}

function normalizeState(value: Partial<LiftState>): LiftState {
  const fallback = defaultState();
  const workouts = Array.isArray(value.workouts) && value.workouts.length > 0 ? value.workouts : fallback.workouts;
  const schedule = { ...fallback.schedule, ...(value.schedule ?? {}) };
  const entries = value.entries ?? {};

  for (const day of dayKeys) {
    if (schedule[day] && !workouts.some((workout) => workout.id === schedule[day])) {
      schedule[day] = null;
    }
  }

  return { version: 1, workouts, schedule, entries };
}

function localDate(date: Date) {
  const year = date.getFullYear();
  const month = `${date.getMonth() + 1}`.padStart(2, "0");
  const day = `${date.getDate()}`.padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function parseDate(value: string) {
  const [year, month, day] = value.split("-").map(Number);
  return new Date(year, month - 1, day);
}

function addDays(value: string, amount: number) {
  const date = parseDate(value);
  date.setDate(date.getDate() + amount);
  return localDate(date);
}

function dayKeyForDate(value: string): DayKey {
  const day = parseDate(value).getDay();
  return dayKeys[(day + 6) % 7];
}

function displayDate(value: string) {
  return new Intl.DateTimeFormat(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric"
  }).format(parseDate(value));
}

function getWorkout(state: LiftState, date: string) {
  const workoutId = state.schedule[dayKeyForDate(date)];
  return state.workouts.find((workout) => workout.id === workoutId) ?? null;
}

function emptySets(exercise: Exercise): EntrySet[] {
  return Array.from({ length: Math.max(1, exercise.sets) }, () => ({
    weight: "",
    reps: `${exercise.reps}`,
    done: false
  }));
}

function ensureEntry(state: LiftState, date: string): JournalEntry {
  const existing = state.entries[date] ?? {
    date,
    bodyWeight: "",
    notes: "",
    exercises: {}
  };
  const workout = getWorkout(state, date);

  if (!workout) {
    return existing;
  }

  return {
    ...existing,
    exercises: workout.exercises.reduce<Record<string, { sets: EntrySet[] }>>((acc, exercise) => {
      acc[exercise.id] = existing.exercises[exercise.id] ?? { sets: emptySets(exercise) };
      return acc;
    }, { ...existing.exercises })
  };
}

function entryCompletion(entry: JournalEntry, workout: Workout | null) {
  if (!workout) {
    return 0;
  }

  const sets = workout.exercises.flatMap((exercise) => entry.exercises[exercise.id]?.sets ?? []);
  if (sets.length === 0) {
    return 0;
  }

  return sets.filter((set) => set.done).length / sets.length;
}

function numberValue(value: string) {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

export function App() {
  const today = useMemo(() => localDate(new Date()), []);
  const [view, setView] = useState<View>("journal");
  const [state, setState] = useState<LiftState>(() => defaultState());
  const [selectedDate, setSelectedDate] = useState(today);
  const [selectedWorkoutId, setSelectedWorkoutId] = useState<string | null>(null);
  const [metric, setMetric] = useState("bodyWeight");
  const [loading, setLoading] = useState(true);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [showSaveNotice, setShowSaveNotice] = useState(false);
  const [dark, setDark] = useState(() => localStorage.getItem("lift-theme") === "dark");

  useEffect(() => {
    document.body.classList.toggle("dark", dark);
    localStorage.setItem("lift-theme", dark ? "dark" : "light");
  }, [dark]);

  useEffect(() => {
    let active = true;
    loadState()
      .then((nextState) => {
        if (!active) return;
        const normalized = normalizeState(nextState);
        setState(normalized);
        setSelectedWorkoutId(normalized.workouts[0]?.id ?? null);
      })
      .catch(() => {
        if (!active) return;
        setSelectedWorkoutId(state.workouts[0]?.id ?? null);
      })
      .finally(() => {
        if (active) setLoading(false);
      });

    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (!dirty || loading) return;

    setSaving("saving");
    const timeout = window.setTimeout(() => {
      saveState(state)
        .then(() => {
          setDirty(false);
          setSaving("saved");
          setShowSaveNotice(true);
        })
        .catch(() => setSaving("error"));
    }, 650);

    return () => window.clearTimeout(timeout);
  }, [dirty, loading, state]);

  useEffect(() => {
    if (!showSaveNotice) return;

    const timeout = window.setTimeout(() => setShowSaveNotice(false), 1800);
    return () => window.clearTimeout(timeout);
  }, [showSaveNotice]);

  const dateWindow = useMemo(
    () => Array.from({ length: 14 }, (_, index) => addDays(today, index - 6)),
    [today]
  );
  const entry = ensureEntry(state, selectedDate);
  const workout = getWorkout(state, selectedDate);
  const selectedWorkout = state.workouts.find((item) => item.id === selectedWorkoutId) ?? state.workouts[0] ?? null;
  const allExercises = state.workouts.flatMap((item) =>
    item.exercises.map((exercise) => ({ workout: item.name, exercise }))
  );
  const workoutOptions = useMemo<SelectOption[]>(
    () => [
      { value: "", label: "Rest" },
      ...state.workouts.map((workoutItem, index) => ({
        value: workoutItem.id,
        label: workoutItem.name.trim() || `Workout ${index + 1}`
      }))
    ],
    [state.workouts]
  );
  const metricOptions = useMemo<SelectOption[]>(
    () => [
      { value: "bodyWeight", label: "Body weight" },
      ...allExercises.map(({ workout: workoutName, exercise }, index) => ({
        value: exercise.id,
        label: `${workoutName.trim() || "Workout"} / ${exercise.name.trim() || `Exercise ${index + 1}`}`
      }))
    ],
    [allExercises]
  );
  const chartData = useMemo(() => {
    return Object.values(state.entries)
      .sort((a, b) => a.date.localeCompare(b.date))
      .map((journalEntry) => {
        if (metric === "bodyWeight") {
          const value = numberValue(journalEntry.bodyWeight);
          return value === null ? null : { date: journalEntry.date.slice(5), value };
        }

        const sets = journalEntry.exercises[metric]?.sets ?? [];
        const best = sets.reduce<number | null>((max, set) => {
          const value = numberValue(set.weight);
          if (value === null) return max;
          return max === null ? value : Math.max(max, value);
        }, null);

        return best === null ? null : { date: journalEntry.date.slice(5), value: best };
      })
      .filter((item): item is { date: string; value: number } => item !== null);
  }, [metric, state.entries]);

  function commit(nextState: LiftState) {
    setState(nextState);
    setDirty(true);
  }

  function updateEntry(updater: (entry: JournalEntry) => JournalEntry) {
    const nextEntry = updater(ensureEntry(state, selectedDate));
    commit({
      ...state,
      entries: {
        ...state.entries,
        [selectedDate]: nextEntry
      }
    });
  }

  function updateSet(exerciseId: string, setIndex: number, patch: Partial<EntrySet>) {
    updateEntry((current) => {
      const currentExercise = current.exercises[exerciseId] ?? { sets: [] };
      const sets = currentExercise.sets.map((set, index) =>
        index === setIndex ? { ...set, ...patch } : set
      );
      return {
        ...current,
        exercises: {
          ...current.exercises,
          [exerciseId]: { sets }
        }
      };
    });
  }

  function addWorkout() {
    const workout: Workout = {
      id: id(),
      name: "",
      exercises: [{ id: id(), name: "", sets: 3, reps: 8 }]
    };
    commit({ ...state, workouts: [...state.workouts, workout] });
    setSelectedWorkoutId(workout.id);
  }

  function updateWorkout(workoutId: string, patch: Partial<Workout>) {
    commit({
      ...state,
      workouts: state.workouts.map((workout) =>
        workout.id === workoutId ? { ...workout, ...patch } : workout
      )
    });
  }

  function deleteWorkout(workoutId: string) {
    const workouts = state.workouts.filter((workout) => workout.id !== workoutId);
    const schedule = { ...state.schedule };
    for (const day of dayKeys) {
      if (schedule[day] === workoutId) schedule[day] = null;
    }
    commit({ ...state, workouts, schedule });
    setSelectedWorkoutId(workouts[0]?.id ?? null);
  }

  function updateExercise(workoutId: string, exerciseId: string, patch: Partial<Exercise>) {
    commit({
      ...state,
      workouts: state.workouts.map((workout) =>
        workout.id === workoutId
          ? {
              ...workout,
              exercises: workout.exercises.map((exercise) =>
                exercise.id === exerciseId ? { ...exercise, ...patch } : exercise
              )
            }
          : workout
      )
    });
  }

  function addExercise(workoutId: string) {
    commit({
      ...state,
      workouts: state.workouts.map((workout) =>
        workout.id === workoutId
          ? {
              ...workout,
              exercises: [
                ...workout.exercises,
                { id: id(), name: "", sets: 3, reps: 8 }
              ]
            }
          : workout
      )
    });
  }

  function deleteExercise(workoutId: string, exerciseId: string) {
    commit({
      ...state,
      workouts: state.workouts.map((workout) =>
        workout.id === workoutId
          ? {
              ...workout,
              exercises: workout.exercises.filter((exercise) => exercise.id !== exerciseId)
            }
          : workout
      )
    });
  }

  return (
    <div className="app-shell">
      <header className="topbar">
        <button className="brand" onClick={() => setView("journal")} aria-label="Open journal">
          lift
        </button>
        <nav className="nav-actions" aria-label="Primary">
          <IconButton active={view === "journal"} label="Journal" onClick={() => setView("journal")}>
            <CalendarDays size={18} />
          </IconButton>
          <IconButton active={view === "plan"} label="Plan" onClick={() => setView("plan")}>
            <Dumbbell size={18} />
          </IconButton>
          <IconButton active={view === "progress"} label="Progress" onClick={() => setView("progress")}>
            <LineChart size={18} />
          </IconButton>
          <IconButton label="Theme" onClick={() => setDark((value) => !value)}>
            {dark ? <Sun size={18} /> : <Moon size={18} />}
          </IconButton>
        </nav>
      </header>

      <main className="main">
        {loading ? (
          <div className="empty-state">Loading journal</div>
        ) : (
          <>
            <div className="status-row">
              <span>{saving === "saving" ? "Saving" : saving === "error" ? "Save failed" : saving === "saved" ? "Saved" : "Ready"}</span>
              {saving === "saved" && <Check size={14} />}
              {saving === "saving" && <Save size={14} />}
            </div>

            {view === "journal" && (
              <section className="view journal-view">
                <div className="date-rail" aria-label="Date selector">
                  {dateWindow.map((date) => {
                    const itemWorkout = getWorkout(state, date);
                    const itemEntry = ensureEntry(state, date);
                    const completion = entryCompletion(itemEntry, itemWorkout);

                    return (
                      <button
                        key={date}
                        className={`date-pill ${date === selectedDate ? "active" : ""}`}
                        onClick={() => setSelectedDate(date)}
                      >
                        <span>{date === today ? "Today" : dayLabels[dayKeyForDate(date)]}</span>
                        <strong>{parseDate(date).getDate()}</strong>
                        <i style={{ width: `${Math.round(completion * 100)}%` }} />
                      </button>
                    );
                  })}
                </div>

                <section className="day-summary">
                  <div>
                    <p className="eyebrow">{displayDate(selectedDate)}</p>
                    <h1>{workout?.name.trim() || "Rest day"}</h1>
                  </div>
                  <label className="compact-field">
                    <span>Weight</span>
                    <input
                      inputMode="decimal"
                      value={entry.bodyWeight}
                      onChange={(event) =>
                        updateEntry((current) => ({ ...current, bodyWeight: event.target.value }))
                      }
                      placeholder="lbs"
                    />
                  </label>
                </section>

                {workout ? (
                  <div className="exercise-list">
                    {workout.exercises.map((exercise) => {
                      const exerciseEntry = entry.exercises[exercise.id] ?? { sets: emptySets(exercise) };

                      return (
                        <section className="exercise-block" key={exercise.id}>
                          <div className="exercise-head">
                            <div>
                              <h2>{exercise.name.trim() || "Untitled exercise"}</h2>
                              <p>{exercise.sets} x {exercise.reps}</p>
                            </div>
                          </div>
                          <div className="set-grid">
                            <span>Set</span>
                            <span>Weight</span>
                            <span>Reps</span>
                            <span>Done</span>
                            {exerciseEntry.sets.map((set, index) => (
                              <div className="set-row" key={`${exercise.id}-${index}`}>
                                <strong>{index + 1}</strong>
                                <input
                                  inputMode="decimal"
                                  value={set.weight}
                                  onChange={(event) => updateSet(exercise.id, index, { weight: event.target.value })}
                                  placeholder="lbs"
                                  aria-label={`${exercise.name} set ${index + 1} weight`}
                                />
                                <input
                                  inputMode="numeric"
                                  value={set.reps}
                                  onChange={(event) => updateSet(exercise.id, index, { reps: event.target.value })}
                                  placeholder="0"
                                  aria-label={`${exercise.name} set ${index + 1} reps`}
                                />
                                <button
                                  className={`check-button ${set.done ? "active" : ""}`}
                                  onClick={() => updateSet(exercise.id, index, { done: !set.done })}
                                  aria-label={`${exercise.name} set ${index + 1} done`}
                                >
                                  <Check size={16} />
                                </button>
                              </div>
                            ))}
                          </div>
                        </section>
                      );
                    })}
                  </div>
                ) : (
                  <div className="empty-state">No workout scheduled</div>
                )}

                <label className="notes-field">
                  <span>Notes</span>
                  <textarea
                    value={entry.notes}
                    onChange={(event) =>
                      updateEntry((current) => ({ ...current, notes: event.target.value }))
                    }
                    placeholder="Notes"
                  />
                </label>
              </section>
            )}

            {view === "plan" && (
              <section className="view plan-view">
                <section className="schedule-grid">
                  {dayKeys.map((day) => (
                    <div className="schedule-item" key={day}>
                      <span>{dayLabels[day]}</span>
                      <CustomSelect
                        value={state.schedule[day] ?? ""}
                        options={workoutOptions}
                        onChange={(event) =>
                          commit({
                            ...state,
                            schedule: {
                              ...state.schedule,
                              [day]: event || null
                            }
                          })
                        }
                      />
                    </div>
                  ))}
                </section>

                <section className="workout-editor">
                  <div className="workout-list">
                    {state.workouts.map((workoutItem) => (
                      <button
                        key={workoutItem.id}
                        className={workoutItem.id === selectedWorkout?.id ? "active" : ""}
                        onClick={() => setSelectedWorkoutId(workoutItem.id)}
                      >
                        {workoutItem.name.trim() || "Untitled"}
                      </button>
                    ))}
                    <button onClick={addWorkout}>
                      <Plus size={16} />
                    </button>
                  </div>

                  {selectedWorkout && (
                    <div className="editor-panel">
                      <div className="editor-title">
                        <input
                          value={selectedWorkout.name}
                          onChange={(event) =>
                            updateWorkout(selectedWorkout.id, { name: event.target.value })
                          }
                          placeholder="Workout name"
                        />
                        <button
                          className="icon-danger"
                          onClick={() => deleteWorkout(selectedWorkout.id)}
                          aria-label="Delete workout"
                        >
                          <Trash2 size={17} />
                        </button>
                      </div>

                      <div className="exercise-editor-list">
                        {selectedWorkout.exercises.map((exercise) => (
                          <div className="exercise-editor" key={exercise.id}>
                            <input
                              value={exercise.name}
                              onChange={(event) =>
                                updateExercise(selectedWorkout.id, exercise.id, {
                                  name: event.target.value
                                })
                              }
                              placeholder="Exercise"
                              aria-label="Exercise name"
                            />
                            <input
                              inputMode="numeric"
                              value={exercise.sets}
                              onChange={(event) =>
                                updateExercise(selectedWorkout.id, exercise.id, {
                                  sets: Math.max(1, Number(event.target.value) || 1)
                                })
                              }
                              placeholder="Sets"
                              aria-label="Sets"
                            />
                            <input
                              inputMode="numeric"
                              value={exercise.reps}
                              onChange={(event) =>
                                updateExercise(selectedWorkout.id, exercise.id, {
                                  reps: Math.max(1, Number(event.target.value) || 1)
                                })
                              }
                              placeholder="Reps"
                              aria-label="Reps"
                            />
                            <button
                              className="icon-danger"
                              onClick={() => deleteExercise(selectedWorkout.id, exercise.id)}
                              aria-label="Delete exercise"
                            >
                              <Trash2 size={16} />
                            </button>
                          </div>
                        ))}
                      </div>

                      <button className="secondary-action" onClick={() => addExercise(selectedWorkout.id)}>
                        <Plus size={16} />
                        <span>Exercise</span>
                      </button>
                    </div>
                  )}
                </section>
              </section>
            )}

            {view === "progress" && (
              <section className="view progress-view">
                <div className="progress-controls">
                  <CustomSelect value={metric} options={metricOptions} onChange={setMetric} />
                </div>

                <div className="chart-wrap">
                  {chartData.length > 1 ? (
                    <ResponsiveContainer width="100%" height="100%">
                      <ReLineChart data={chartData} margin={{ top: 16, right: 12, left: 0, bottom: 8 }}>
                        <CartesianGrid strokeDasharray="3 3" stroke="var(--line-muted)" />
                        <XAxis dataKey="date" stroke="var(--text-muted)" tickLine={false} axisLine={false} />
                        <YAxis stroke="var(--text-muted)" tickLine={false} axisLine={false} width={42} />
                        <Tooltip
                          contentStyle={{
                            background: "var(--surface)",
                            border: "1px solid var(--border)",
                            borderRadius: 8,
                            color: "var(--text)"
                          }}
                        />
                        <Line type="monotone" dataKey="value" stroke="var(--accent)" strokeWidth={2} dot={{ r: 3 }} />
                      </ReLineChart>
                    </ResponsiveContainer>
                  ) : (
                    <div className="empty-state">Add a few entries to draw progress</div>
                  )}
                </div>
              </section>
            )}
          </>
        )}
      </main>
      <div className={`save-toast ${showSaveNotice ? "visible" : ""}`} role="status" aria-live="polite">
        <Check size={15} />
        <span>Saved</span>
      </div>
    </div>
  );
}

function IconButton({
  active,
  children,
  label,
  onClick
}: {
  active?: boolean;
  children: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      className={`icon-button ${active ? "active" : ""}`}
      onClick={onClick}
      title={label}
      aria-label={label}
    >
      {children}
    </button>
  );
}

function CustomSelect({
  value,
  options,
  onChange
}: {
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const selected = options.find((option) => option.value === value) ?? options[0];

  return (
    <div className={`custom-select ${open ? "open" : ""}`} onBlur={() => window.setTimeout(() => setOpen(false), 120)}>
      <button
        type="button"
        className="custom-select-trigger"
        onClick={() => setOpen((current) => !current)}
        aria-haspopup="listbox"
        aria-expanded={open}
      >
        <span>{selected?.label ?? "Select"}</span>
        <ChevronDown size={15} />
      </button>
      {open && (
        <div className="custom-select-menu" role="listbox">
          {options.map((option) => (
            <button
              key={option.value}
              type="button"
              className={option.value === value ? "selected" : ""}
              onClick={() => {
                onChange(option.value);
                setOpen(false);
              }}
              role="option"
              aria-selected={option.value === value}
            >
              <span>{option.label}</span>
              {option.value === value && <Check size={14} />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
