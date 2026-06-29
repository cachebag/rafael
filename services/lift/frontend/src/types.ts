export type DayKey = "mon" | "tue" | "wed" | "thu" | "fri" | "sat" | "sun";

export interface Exercise {
  id: string;
  name: string;
  sets: number;
  reps: number;
}

export interface Workout {
  id: string;
  name: string;
  exercises: Exercise[];
}

export interface ExerciseSnapshot {
  id: string;
  name: string;
  sets: number;
  reps: number;
}

export interface WorkoutSnapshot {
  id: string;
  name: string;
  exercises: ExerciseSnapshot[];
}

export interface EntrySet {
  weight: string;
  reps: string;
  done: boolean;
}

export interface ExerciseEntry {
  sets: EntrySet[];
}

export interface JournalEntry {
  date: string;
  workoutId?: string | null;
  workoutSnapshot?: WorkoutSnapshot | null;
  bodyWeight: string;
  notes: string;
  exercises: Record<string, ExerciseEntry>;
}

export interface LiftState {
  version: 1;
  workouts: Workout[];
  schedule: Record<DayKey, string | null>;
  entries: Record<string, JournalEntry>;
}
