import { ReactNode, useEffect } from "react";
import { X } from "lucide-react";

interface ModalProps {
  isOpen: boolean;
  onClose: () => void;
  children: ReactNode;
  title?: string;
}

export function Modal({ isOpen, onClose, children, title }: ModalProps) {
  useEffect(() => {
    if (isOpen) {
      document.body.style.overflow = "hidden";
    } else {
      document.body.style.overflow = "";
    }
    return () => {
      document.body.style.overflow = "";
    };
  }, [isOpen]);

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div
        className="absolute inset-0 bg-charcoal-900/50 backdrop-blur-sm"
        onClick={onClose}
      />
      <div className="relative bg-sand-50 dark:bg-charcoal-900 p-6 shadow-xl max-w-lg w-full mx-4 max-h-[90vh] overflow-y-auto">
        <div className="flex items-center justify-between mb-4">
          {title && (
            <h2 className="text-lg font-semibold text-charcoal-800 dark:text-sand-100">
              {title}
            </h2>
          )}
          <button
            onClick={onClose}
            className="p-1 hover:bg-sand-200 dark:hover:bg-charcoal-800 transition-colors ml-auto"
          >
            <X size={20} />
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}

